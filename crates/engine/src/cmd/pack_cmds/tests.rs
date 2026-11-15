use serde_json::json;

use super::*;
use super::super::pack::{self, PackTexture};

fn session() -> Session {
    Session::new()
}

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A pack folder in a temporary directory, deleted with the test.
struct TempPack(tempfile::TempDir);

impl TempPack {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn root(&self) -> String {
        self.0.path().display().to_string()
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.0.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn open(&self, s: &mut Session) {
        s.execute("pack.open", json!({"path": self.root()})).unwrap();
    }
}

fn mcmeta(description: &str, format: i64) -> Vec<u8> {
    json!({"pack": {"pack_format": format, "description": description}}).to_string().into_bytes()
}

fn texture(rel: &str) -> PackTexture {
    let (namespace, category) = pack::classify(rel);
    PackTexture { rel: rel.into(), namespace, category }
}

// ---------- scanning ----------

#[test]
fn a_scan_lists_only_pngs_with_their_namespace_and_category() {
    let pack_dir = TempPack::new();
    pack_dir.write("pack.mcmeta", &mcmeta("My Pack", 34));
    pack_dir.write("assets/minecraft/textures/block/stone.png", b"png");
    pack_dir.write("assets/minecraft/textures/block/dirt.png", b"png");
    pack_dir.write("assets/minecraft/textures/item/stick.png", b"png");
    pack_dir.write("assets/minecraft/textures/entity/creeper/creeper.png", b"png");
    pack_dir.write("assets/minecraft/textures/block/animated.png.mcmeta", b"{}");
    pack_dir.write("assets/minecraft/lang/en_us.json", b"{}");
    pack_dir.write("pack.png", b"icon");
    pack_dir.write("notes.txt", b"readme");
    pack_dir.write(".git/config", b"[core]");
    let mut s = session();
    let r = s.execute("pack.open", json!({"path": pack_dir.root()})).unwrap();
    assert_eq!(r["name"], pack_dir.0.path().file_name().unwrap().to_string_lossy().into_owned());
    assert_eq!(r["description"], "My Pack");
    assert_eq!(r["packFormat"], 34);
    assert_eq!(r["textureCount"], 5);
    assert_eq!(r["categories"]["block"], 2);
    assert_eq!(r["categories"]["item"], 1);
    assert_eq!(r["categories"]["entity"], 1);
    assert_eq!(r["categories"]["icon"], 1);
    assert!(r["warnings"].as_array().unwrap().is_empty());
    // pack.open does not return the list; check it through the state (see the list tests below).
    let mut s = session();
    pack_dir.open(&mut s);
    assert_eq!(
        s.pack.textures,
        vec![
            texture("assets/minecraft/textures/block/dirt.png"),
            texture("assets/minecraft/textures/block/stone.png"),
            texture("assets/minecraft/textures/entity/creeper/creeper.png"),
            texture("pack.png"),
            texture("assets/minecraft/textures/item/stick.png"),
        ]
    );
}

#[test]
fn a_pack_without_mcmeta_still_opens_with_a_warning() {
    let pack_dir = TempPack::new();
    pack_dir.write("assets/minecraft/textures/block/stone.png", b"png");
    let mut s = session();
    let r = s.execute("pack.open", json!({"path": pack_dir.root()})).unwrap();
    assert_eq!(r["packFormat"], json!(null));
    assert_eq!(r["warnings"], json!(["no pack.mcmeta; a resource pack needs one"]));
}

#[test]
fn mcmeta_description_variants_and_a_broken_file_become_warnings() {
    let pack_dir = TempPack::new();
    pack_dir.write("assets/minecraft/textures/block/stone.png", b"png");
    // The JSON text-component form: {text: ...}.
    pack_dir.write("pack.mcmeta", br#"{"pack": {"pack_format": 34, "description": {"text": "Fancy", "color": "red"}}}"#);
    let mut s = session();
    let r = s.execute("pack.open", json!({"path": pack_dir.root()})).unwrap();
    assert_eq!(r["description"], "Fancy");
    assert!(r["warnings"].as_array().unwrap().is_empty());
    // A description PhotoCraft cannot read (an empty text component) warns but opens.
    pack_dir.write("pack.mcmeta", br#"{"pack": {"pack_format": 34, "description": {"color": "red"}}}"#);
    let mut s = session();
    let r = s.execute("pack.open", json!({"path": pack_dir.root()})).unwrap();
    assert_eq!(r["description"], "");
    assert_eq!(r["warnings"], json!(["pack.mcmeta's description is not plain text"]));    // Broken JSON warns too.
    pack_dir.write("pack.mcmeta", b"{not json");
    let mut s = session();
    let r = s.execute("pack.open", json!({"path": pack_dir.root()})).unwrap();
    assert_eq!(r["warnings"], json!(["pack.mcmeta is not valid JSON"]));
    // A suspiciously large mcmeta is refused without being parsed.
    pack_dir.write("pack.mcmeta", &vec![b' '; 2 << 20]);
    let mut s = session();
    let r = s.execute("pack.open", json!({"path": pack_dir.root()})).unwrap();
    assert_eq!(r["warnings"], json!(["pack.mcmeta is too large to read"]));
}

#[test]
fn an_empty_pack_opens_with_a_warning() {
    let pack_dir = TempPack::new();
    pack_dir.write("pack.mcmeta", &mcmeta("Empty", 34));
    let mut s = session();
    let r = s.execute("pack.open", json!({"path": pack_dir.root()})).unwrap();
    assert_eq!(r["textureCount"], 0);
    assert_eq!(r["warnings"], json!(["the pack holds no .png textures"]));
}

#[test]
fn opening_refuses_bad_paths() {
    let mut s = session();
    let pack_dir = TempPack::new();
    std::fs::write(pack_dir.0.path().join("file.txt"), b"x").unwrap();
    assert!(s.execute("pack.open", json!({})).is_err(), "no path");
    assert!(s.execute("pack.open", json!({"path": ""})).is_err(), "empty path");
    assert!(s.execute("pack.open", json!({"path": 3})).is_err(), "a number");
    assert!(s
        .execute("pack.open", json!({"path": pack_dir.0.path().join("file.txt").display().to_string()}))
        .is_err(), "a file, not a folder");
    assert!(s.execute("pack.open", json!({"path": pack_dir.0.path().join("gone").display().to_string()})).is_err(), "missing");
}

#[test]
fn a_scan_hits_its_per_folder_cap() {
    let pack_dir = TempPack::new();
    for i in 0..pack::MAX_DIR_ENTRIES + 1 {
        pack_dir.write(&format!("d{i}.png"), b"png");
    }
    let mut s = session();
    assert!(s.execute("pack.open", json!({"path": pack_dir.root()})).is_err());
}

// ---------- path hygiene ----------

#[test]
fn relative_paths_must_be_clean() {
    for bad in [
        "",                                     // empty
        "/assets/minecraft/textures/block/x.png", // absolute
        "assets/..\\textures/x.png",            // backslash
        "assets//textures/x.png",               // empty part
        "assets/../etc/passwd",                 // ..
        "assets/./x.png",                       // .
        "assets/minecraft/textures/.hidden.png", // dot part
        "assets/minecraft/textures/x.png/",     // trailing slash
        &("a/".repeat(300) + "x.png"),          // too long
        "assets/minecraft/textures/x\u{7}.png", // control character
    ] {
        assert!(pack::check_rel(bad).is_err(), "`{bad}` must be refused");
    }
    for good in [
        "assets/minecraft/textures/block/stone.png",
        "assets/my-pack_2.test/textures/gui/title/panorama_0.png",
        "pack.png",
        "assets/minecraft/textures/block/a b.png",
    ] {
        assert!(pack::check_rel(good).is_ok(), "`{good}` must be accepted");
    }
}

#[test]
fn new_texture_paths_must_be_pack_textures() {
    assert!(pack::check_texture_rel("assets/minecraft/textures/block/stone.png").is_ok());
    assert!(pack::check_texture_rel("pack.png").is_err(), "not under assets/<ns>/textures/");
    assert!(pack::check_texture_rel("assets/minecraft/blocks/stone.png").is_err(), "no textures folder");
    assert!(pack::check_texture_rel("assets/minecraft/textures/stone.png").is_err(), "needs a category folder");
    assert!(pack::check_texture_rel("assets/Minecraft/textures/block/stone.png").is_err(), "uppercase namespace");
    assert!(pack::check_texture_rel("assets/-mc/textures/block/stone.png").is_err(), "namespace may not start with -");
    assert!(pack::check_texture_rel("assets/minecraft/textures/block/stone.txt").is_err(), "not .png");
    assert!(pack::check_texture_rel("textures/block/stone.png").is_err(), "no assets/ namespace");
}

// ---------- listing ----------

#[test]
fn list_filters_by_category_and_search() {
    let pack_dir = TempPack::new();
    pack_dir.write("pack.mcmeta", &mcmeta("P", 34));
    pack_dir.write("assets/minecraft/textures/block/stone.png", b"png");
    pack_dir.write("assets/minecraft/textures/block/stone_deepslate.png", b"png");
    pack_dir.write("assets/minecraft/textures/item/stone_sword.png", b"png");
    let mut s = session();
    pack_dir.open(&mut s);
    let r = s.execute("pack.list", json!({})).unwrap();
    assert_eq!(r["total"], 3);
    assert_eq!(r["count"], 3);
    assert_eq!(r["textures"][0]["path"], "assets/minecraft/textures/block/stone.png");
    assert_eq!(r["textures"][0]["namespace"], "minecraft");
    assert_eq!(r["textures"][0]["category"], "block");
    let r = s.execute("pack.list", json!({"category": "item"})).unwrap();
    assert_eq!(r["total"], 1);
    assert_eq!(r["textures"][0]["path"], "assets/minecraft/textures/item/stone_sword.png");
    let r = s.execute("pack.list", json!({"category": "gui"})).unwrap();
    assert_eq!(r["total"], 0);
    let r = s.execute("pack.list", json!({"search": "STONE"})).unwrap();
    assert_eq!(r["total"], 3, "the search is case-insensitive");
    let r = s.execute("pack.list", json!({"search": "sword"})).unwrap();
    assert_eq!(r["total"], 1);
    let r = s.execute("pack.list", json!({"limit": 1, "offset": 1})).unwrap();
    assert_eq!(r["total"], 3);
    assert_eq!(r["count"], 1);
    assert_eq!(r["textures"][0]["path"], "assets/minecraft/textures/block/stone_deepslate.png");
    let r = s.execute("pack.list", json!({"offset": 99})).unwrap();
    assert_eq!(r["count"], 0, "an offset past the end is empty, not an error");
    assert!(s.execute("pack.list", json!({"limit": 0})).is_err());
    assert!(s.execute("pack.list", json!({"limit": 5001})).is_err());
    assert!(s.execute("pack.list", json!({"search": "x".repeat(201)})).is_err());
}

#[test]
fn info_and_list_refuse_without_a_pack() {
    let mut s = session();
    let r = s.execute("pack.info", json!({})).unwrap();
    assert_eq!(r["open"], false);
    assert!(s.execute("pack.list", json!({})).is_err(), "no pack open");
    assert!(s.execute("pack.openTexture", json!({"path": "x.png"})).is_err());
    assert!(s.execute("pack.newTexture", json!({"path": "assets/minecraft/textures/block/x.png"})).is_err());
    assert!(s.execute("pack.saveTexture", json!({})).is_err());
}

// ---------- creating and saving textures ----------

#[test]
fn a_new_texture_creates_a_transparent_document_bound_to_the_pack() {
    let pack_dir = TempPack::new();
    pack_dir.write("pack.mcmeta", &mcmeta("P", 34));
    let mut s = session();
    pack_dir.open(&mut s);
    let r = s.execute("pack.newTexture", json!({"path": "assets/minecraft/textures/block/stone.png", "width": 32, "height": 16})).unwrap();
    assert_eq!(r["width"], 32);
    assert_eq!(r["height"], 16);
    let rel = r["packPath"].as_str().unwrap();
    let doc = s.active().unwrap();
    assert_eq!(doc.doc.name, "stone");
    assert_eq!(doc.doc.size.width, 32);
    assert_eq!(doc.doc.size.height, 16);
    assert_eq!(s.pack.target_of(doc.doc.id), Some(rel));
    assert_eq!(
        r["absolutePath"],
        pack_dir.0.path().join("assets/minecraft/textures/block/stone.png").display().to_string()
    );
    // Height defaults to width.
    let r = s.execute("pack.newTexture", json!({"path": "assets/minecraft/textures/item/stick.png", "width": 64})).unwrap();
    assert_eq!(r["height"], 64);
    assert_eq!(s.active().unwrap().doc.size.width, 64);
    // Bad paths and sizes are refused without creating a document.
    let docs = s.documents().len();
    assert!(s.execute("pack.newTexture", json!({"path": "textures/block/stone.png"})).is_err());
    assert!(s.execute("pack.newTexture", json!({"path": "assets/minecraft/textures/block/stone.png", "width": 0})).is_err());
    assert!(s.execute("pack.newTexture", json!({"path": "assets/minecraft/textures/block/stone.png", "width": 4097})).is_err());
    assert!(s.execute("pack.newTexture", json!({"path": "assets/minecraft/textures/block/stone.png", "width": "big"})).is_err());
    assert_eq!(s.documents().len(), docs, "a refused creation leaves no document");
}

#[test]
fn saving_a_new_texture_writes_the_pack_file_and_its_folder() {
    let pack_dir = TempPack::new();
    pack_dir.write("pack.mcmeta", &mcmeta("P", 34));
    let mut s = session();
    pack_dir.open(&mut s);
    let rel = "assets/minecraft/textures/block/stone.png";
    s.execute("pack.newTexture", json!({"path": rel, "width": 16, "height": 16})).unwrap();
    s.execute("pack.saveTexture", json!({})).unwrap();
    let path = pack_dir.0.path().join(rel);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..8], PNG_MAGIC, "the texture is written as a PNG");
    let saved = s.active().unwrap().path.clone().unwrap_or_default();
    assert_eq!(saved, path.display().to_string(), "the file becomes the document's own");
    assert!(!s.active().unwrap().is_dirty(), "a saved texture is not dirty");
    // An explicit `path` saves a document that is not bound, and creates its folder.
    let rel2 = "assets/minecraft/textures/item/new_folder/stick.png";
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("pack.saveTexture", json!({"path": rel2})).unwrap();
    assert_eq!(&std::fs::read(pack_dir.0.path().join(rel2)).unwrap()[..8], PNG_MAGIC);
}

#[test]
fn save_texture_refuses_an_unbound_document_without_a_path() {
    let pack_dir = TempPack::new();
    pack_dir.write("pack.mcmeta", &mcmeta("P", 34));
    let mut s = session();
    pack_dir.open(&mut s);
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert!(s.execute("pack.saveTexture", json!({})).is_err(), "no document and no path");
    assert!(s.execute("pack.saveTexture", json!({"path": "../outside.png"})).is_err());
}

#[test]
fn reopening_a_pack_clears_the_texture_targets() {
    let pack_dir = TempPack::new();
    pack_dir.write("pack.mcmeta", &mcmeta("P", 34));
    let mut s = session();
    pack_dir.open(&mut s);
    s.execute("pack.newTexture", json!({"path": "assets/minecraft/textures/block/stone.png"})).unwrap();
    assert_eq!(s.pack.targets.len(), 1);
    pack_dir.open(&mut s);
    assert!(s.pack.targets.is_empty(), "a fresh scan has no stale targets");
    s.execute("pack.newTexture", json!({"path": "assets/minecraft/textures/block/stone.png"})).unwrap();
    s.execute("pack.close", json!({})).unwrap();
    assert!(s.pack.pack.is_none());
    assert!(s.pack.targets.is_empty());
    assert!(s.pack.textures.is_empty());
    let r = s.execute("pack.close", json!({})).unwrap();
    assert_eq!(r["closed"], false, "closing an empty workspace says so");
}

// ---------- the full round trip: create, save, reopen, open ----------

#[test]
fn a_created_texture_reopens_from_the_pack_bound_to_its_path() {
    let pack_dir = TempPack::new();
    pack_dir.write("pack.mcmeta", &mcmeta("P", 34));
    let mut s = session();
    pack_dir.open(&mut s);
    let rel = "assets/minecraft/textures/block/stone.png";
    s.execute("pack.newTexture", json!({"path": rel, "width": 24, "height": 8})).unwrap();
    s.execute("pack.saveTexture", json!({})).unwrap();
    // Close the texture, close the pack, reopen the pack: the file is there and opens.
    s.execute("file.close", json!({"save": "discard"})).unwrap();
    pack_dir.open(&mut s);
    assert_eq!(s.pack.textures.len(), 1);
    s.execute("pack.openTexture", json!({"path": rel})).unwrap();
    let doc = s.active().unwrap();
    assert_eq!(doc.doc.name, "stone.png");
    assert_eq!(doc.doc.size.width, 24);
    assert_eq!(doc.doc.size.height, 8);
    assert_eq!(s.pack.target_of(doc.doc.id), Some(rel));
    assert_eq!(doc.path.as_deref(), Some(pack_dir.0.path().join(rel).display().to_string().as_str()));
    // A texture that is not of the open pack is refused.
    assert!(s.execute("pack.openTexture", json!({"path": "assets/minecraft/textures/block/other.png"})).is_err());
    assert!(s.execute("pack.openTexture", json!({"path": "pack.mcmeta"})).is_err(), "not a .png");
}

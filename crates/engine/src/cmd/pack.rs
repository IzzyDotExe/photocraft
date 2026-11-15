//! The Minecraft Java Edition texture pack workspace (Window › Minecraft Pack; the commands are
//! [`crate::pack_cmds`]). State only: which pack folder is open, the textures it holds, and which
//! open documents are pack textures. Everything is derived from the folder on `pack.open` — the
//! pack on disk is the only source of truth, so an external change is picked up by reopening.
//!
//! A pack is a folder with `pack.mcmeta` and textures under `assets/<namespace>/textures/…`.
//! Textures are classified by the vanilla category folders (`block`, `item`, `entity`, `gui`,
//! `environment`, `painting`, `particle`, `misc`, `mob_effect`, `trident`, …); anything else is
//! `other`, and the root `pack.png` is the pack's `icon`.
//!
//! Every filesystem operation here is a desktop capability: on the web `scan` fails, the pack
//! commands refuse (their `enabled` checks), and the workspace stays empty.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use photocraft_doc::DocId;

/// The most textures one pack may hold (a scan stops there and refuses the pack).
pub const MAX_TEXTURES: usize = 20_000;
/// The most files one scan looks at, textures or not (stops a runaway or cyclic tree).
pub const MAX_SCAN_FILES: usize = 100_000;
/// The deepest a scan descends (packs are shallow; this also stops symlink and bind cycles).
const MAX_SCAN_DEPTH: u32 = 12;
/// The most entries one scanned folder may hold.
pub(crate) const MAX_DIR_ENTRIES: usize = 8_000;
/// The largest `pack.mcmeta` read (it is a tiny JSON file; a larger one is refused, not parsed).
const MAX_MCMETA_BYTES: u64 = 1 << 20;
/// The longest accepted pack-relative path, in bytes.
const MAX_REL_BYTES: usize = 512;
/// The largest texture side `pack.newTexture` creates. `file.new`'s own limit is far larger;
/// Minecraft textures are small, and this keeps a mistyped agent path from allocating huge
/// canvases by accident.
pub const MAX_TEXTURE_SIDE: u32 = 4096;

/// One texture in the pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackTexture {
    /// Path relative to the pack root, `/`-separated: `assets/minecraft/textures/block/stone.png`.
    pub rel: String,
    /// The resource namespace (`minecraft` in `assets/minecraft/textures/…`); empty for files
    /// outside `assets/`, such as the root `pack.png`.
    pub namespace: String,
    /// The vanilla texture category: the folder right under `textures/` (`block`, `item`, …),
    /// `icon` for the root `pack.png`, else `other`.
    pub category: String,
}

/// The open pack, as `pack.mcmeta` and the folder name describe it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackInfo {
    /// The pack folder's name.
    pub name: String,
    /// `pack.mcmeta`'s description (its `text` when the JSON text-component form is used).
    pub description: String,
    /// `pack.mcmeta`'s `pack.pack_format`, when present.
    pub pack_format: Option<i64>,
    /// The pack folder, as the user gave it.
    pub root: String,
    /// Notes from opening: no `pack.mcmeta`, an unparsable one, a description PhotoCraft cannot
    /// read, no textures found. Opening still succeeds for all of these.
    pub warnings: Vec<String>,
}

/// The pack workspace held by the session (`Session::pack`).
#[derive(Default)]
pub struct PackState {
    /// The open pack, if any.
    pub pack: Option<PackInfo>,
    /// Its textures, sorted by category then path.
    pub textures: Vec<PackTexture>,
    /// Open documents backed by a pack texture (opened from the pack or created into it),
    /// mapped to the pack-relative path. Entries outlive their documents' DocIds (ids are never
    /// reused), so the map is bounded by the session's document count.
    pub targets: HashMap<DocId, String>,
}

impl PackState {
    /// Scan `root` and replace the workspace with it.
    pub fn open(&mut self, root: &str) -> Result<PackInfo, String> {
        let (info, textures) = scan(root)?;
        self.pack = Some(info.clone());
        self.textures = textures;
        self.targets.clear();
        Ok(info)
    }

    /// Close the workspace.
    pub fn close(&mut self) {
        *self = Self::default();
    }

    /// The absolute path of a pack-relative path, when the workspace is open and the path is a
    /// clean relative path.
    pub fn abs(&self, rel: &str) -> Option<PathBuf> {
        check_rel(rel).ok()?;
        let root = Path::new(&self.pack.as_ref()?.root);
        Some(root.join(rel))
    }

    /// Record that `doc` is the pack texture `rel` (the path is validated first; a bad path is
    /// ignored, as it could only come from PhotoCraft itself).
    pub fn register_target(&mut self, doc: DocId, rel: &str) {
        if check_rel(rel).is_ok() {
            self.targets.insert(doc, rel.to_string());
        }
    }

    /// The pack-relative path `doc` is a texture of, if any.
    pub fn target_of(&self, doc: DocId) -> Option<&str> {
        self.targets.get(&doc).map(String::as_str)
    }
}

/// Validate a user-supplied pack-relative path (`a/b/c.png`): forward slashes only, no leading
/// slash, no empty, `.` or `..` parts, printable ASCII word characters, `-._` and spaces, and at
/// most [`MAX_REL_BYTES`] bytes. Returns the path unchanged on success.
pub fn check_rel(rel: &str) -> Result<(), String> {
    if rel.is_empty() {
        return Err("the path is empty".into());
    }
    if rel.len() > MAX_REL_BYTES {
        return Err(format!("the path is longer than {MAX_REL_BYTES} bytes"));
    }
    if rel.contains('\\') {
        return Err("use `/`, not `\\`, to separate path parts".into());
    }
    if rel.starts_with('/') {
        return Err("the path must be relative to the pack folder".into());
    }
    for part in rel.split('/') {
        if part.is_empty() {
            return Err("the path has an empty part".into());
        }
        if part == "." || part == ".." {
            return Err("the path may not contain `.` or `..` parts".into());
        }
        if part.starts_with('.') {
            return Err("path parts may not start with `.`".into());
        }
        if !part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b' ')) {
            return Err("path parts may only use A-Z, a-z, 0-9, `-`, `_`, `.` and spaces".into());
        }
    }
    Ok(())
}

/// Validate a texture path for `pack.newTexture`: a clean relative path at
/// `assets/<namespace>/textures/<category>/<name>.png` (at least one folder under `textures/`),
/// with a Minecraft namespace (`a-z0-9._-`, not starting with `.` or `-`).
pub fn check_texture_rel(rel: &str) -> Result<(), String> {
    check_rel(rel)?;
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.len() < 5 || parts[0] != "assets" || parts[2] != "textures" {
        return Err("a pack texture lives at assets/<namespace>/textures/<category>/<name>.png".into());
    }
    let ns = parts[1];
    let ok_ns = ns.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && ns.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-'));
    if !ok_ns {
        return Err("the namespace uses a-z, 0-9, `_`, `.` and `-`, starting with an a-z or 0-9".into());
    }
    if !rel.ends_with(".png") {
        return Err("a texture file ends in .png".into());
    }
    Ok(())
}

/// Classify a scanned texture path.
pub(crate) fn classify(rel: &str) -> (String, String) {
    if rel == "pack.png" {
        return (String::new(), "icon".into());
    }
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.len() >= 3 && parts[0] == "assets" {
        let ns = parts[1].to_string();
        if parts.len() >= 4 && parts[2] == "textures" {
            let category = if parts.len() >= 5 { parts[3].to_string() } else { "misc".into() };
            return (ns, category);
        }
        return (ns, "other".into());
    }
    (String::new(), "other".into())
}

/// Scan a pack folder. The pack is not required to be a valid resource pack: a missing or
/// unparsable `pack.mcmeta` is a warning, not an error, so a folder of textures can still be
/// edited (but `pack.open`'s response says so).
#[cfg(not(target_arch = "wasm32"))]
pub fn scan(root: &str) -> Result<(PackInfo, Vec<PackTexture>), String> {
    if root.is_empty() {
        return Err("the pack folder is empty".into());
    }
    let path = Path::new(root);
    let meta = std::fs::metadata(path).map_err(|e| format!("`{root}` could not be opened: {e}"))?;
    if !meta.is_dir() {
        return Err(format!("`{root}` is not a folder"));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.trim_end_matches('/').to_string());
    let (description, pack_format, mut warnings) = read_mcmeta(path)?;
    let mut textures = Vec::new();
    let mut files = 0usize;
    walk(path, Path::new(""), 0, &mut textures, &mut files)?;
    textures.sort_by(|a, b| (&a.category, &a.rel).cmp(&(&b.category, &b.rel)));
    if textures.is_empty() {
        warnings.push("the pack holds no .png textures".into());
    }
    Ok((PackInfo { name, description, pack_format, root: root.to_string(), warnings }, textures))
}

/// The web build has no file system; the pack commands are refused there (see `has_filesystem`).
#[cfg(target_arch = "wasm32")]
pub fn scan(_root: &str) -> Result<(PackInfo, Vec<PackTexture>), String> {
    Err("not available on the web (no file system)".into())
}

/// Read and parse `pack.mcmeta`, returning the description text, the pack format and warnings.
/// A missing file is a warning, not an error, so a folder of textures can still be opened.
#[cfg(not(target_arch = "wasm32"))]
fn read_mcmeta(root: &Path) -> Result<(String, Option<i64>, Vec<String>), String> {
    let path = root.join("pack.mcmeta");
    let meta = match std::fs::metadata(&path) {
        Ok(m) if m.is_file() => m,
        Ok(_) => return Ok((String::new(), None, vec!["pack.mcmeta is not a regular file".into()])),
        Err(_) => return Ok((String::new(), None, vec!["no pack.mcmeta; a resource pack needs one".into()])),
    };
    if meta.len() > MAX_MCMETA_BYTES {
        return Ok((String::new(), None, vec!["pack.mcmeta is too large to read".into()]));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("pack.mcmeta could not be read: {e}"))?;
    let v: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return Ok((String::new(), None, vec!["pack.mcmeta is not valid JSON".into()])),
    };
    let pack = v.get("pack").ok_or_else(|| String::from("pack.mcmeta has no `pack` object"))?;
    let format = pack.get("pack_format").and_then(serde_json::Value::as_i64);
    let mut warnings = Vec::new();
    let description = match pack.get("description") {
        None => {
            warnings.push("pack.mcmeta has no description".into());
            String::new()
        }
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.get("text").and_then(serde_json::Value::as_str).map(String::from).unwrap_or_else(|| {
            warnings.push("pack.mcmeta's description is not plain text".into());
            String::new()
        }),
    };
    Ok((description, format, warnings))
}

/// Walk `dir`, collecting `.png` files as pack textures. Symlinks are skipped (so a symlinked
/// folder cannot cycle); entry order is sorted for a deterministic listing.
#[cfg(not(target_arch = "wasm32"))]
fn walk(dir: &Path, rel_dir: &Path, depth: u32, textures: &mut Vec<PackTexture>, files: &mut usize) -> Result<(), String> {
    if depth > MAX_SCAN_DEPTH {
        return Err(format!("pack folders nest more than {MAX_SCAN_DEPTH} levels deep"));
    }
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
        .map_err(|e| format!("`{}` could not be read: {e}", rel_dir.display()))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("`{}` could not be read: {e}", rel_dir.display()))?;
    if entries.len() > MAX_DIR_ENTRIES {
        return Err(format!("a pack folder holds more than {MAX_DIR_ENTRIES} entries"));
    }
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        *files += 1;
        if *files > MAX_SCAN_FILES {
            return Err(format!("the pack holds more than {MAX_SCAN_FILES} files"));
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        let rel = if rel_dir.as_os_str().is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", rel_dir.display(), name)
        };
        let kind = e.file_type().map_err(|e| format!("`{rel}` could not be read: {e}"))?;
        if kind.is_dir() {
            if name == ".git" {
                continue; // a pack checked out of git is not textures
            }
            walk(&e.path(), Path::new(&rel), depth + 1, textures, files)?;
        } else if kind.is_file() && name.to_ascii_lowercase().ends_with(".png") {
            if textures.len() >= MAX_TEXTURES {
                return Err(format!("the pack holds more than {MAX_TEXTURES} textures"));
            }
            let (namespace, category) = classify(&rel);
            textures.push(PackTexture { rel, namespace, category });
        }
    }
    Ok(())
}

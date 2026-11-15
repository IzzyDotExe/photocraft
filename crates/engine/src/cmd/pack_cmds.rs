//! The Minecraft Java Edition texture pack workspace (Window › Minecraft Pack; see `pack`):
//! open a pack folder, browse its textures, open one as a document, create a new texture into
//! the pack and save it back. A texture opened or created through these commands is bound to
//! its pack path, so `pack.saveTexture` knows where it goes without re-asking.
//!
//! Every command is plain data on the engine session (`Session::pack`), so the UI panel, the
//! control channel, the CLI and MCP all drive the same workspace. The filesystem commands are
//! refused on the web (no file system); the rest work anywhere.

use serde_json::{Value, json};

use super::pack::{self, MAX_TEXTURE_SIDE};
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The most bytes one pack texture file may occupy when `pack.openTexture` reads it (a texture
/// is small; anything larger is not one, and this bounds the read before the import's own
/// memory gate sees it).
const MAX_TEXTURE_FILE_BYTES: u64 = 256 << 20;
/// The most textures `pack.list` returns at once.
const MAX_LIST_LIMIT: u64 = 5000;
/// The longest `pack.list`'s search, in characters.
const MAX_SEARCH_CHARS: usize = 200;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Filesystem access is a desktop capability: the web build has none (like `file.open`).
fn has_filesystem(_: &Session) -> std::result::Result<(), String> {
    if cfg!(target_arch = "wasm32") { Err("not available on the web (no file system)".into()) } else { Ok(()) }
}

/// A pack must be open for a command to mean anything.
fn pack_open(s: &Session) -> std::result::Result<(), String> {
    if s.pack.pack.is_some() { Ok(()) } else { Err("no pack open (run `pack.open` first)".into()) }
}

/// The `path` param: a pack-relative path (see [`pack::check_rel`]), trimmed and non-empty.
fn rel_param(p: &Value, cmd: &str) -> Result<String> {
    let rel = p
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| bad(cmd, "pass `path`, the texture's pack-relative path like assets/minecraft/textures/block/stone.png"))?;
    pack::check_rel(rel).map_err(|e| bad(cmd, e))?;
    Ok(rel.to_string())
}

/// The `width`/`height` param of `pack.newTexture`: a whole number in 1..=[MAX_TEXTURE_SIDE];
/// height defaults to width.
fn side(p: &Value, key: &str, default: u32, cmd: &str) -> Result<u32> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            let n = v
                .as_f64()
                .filter(|f| f.is_finite())
                .ok_or_else(|| bad(cmd, format!("`{key}` must be a number in 1..={MAX_TEXTURE_SIDE}")))?;
            let n = n.round();
            if !(1.0..=f64::from(MAX_TEXTURE_SIDE)).contains(&n) {
                return Err(bad(cmd, format!("`{key}` is 1..={MAX_TEXTURE_SIDE}")));
            }
            Ok(n as u32)
        }
    }
}

/// `pack.list`'s `search`: a case-insensitive substring of the texture path.
fn search_param(p: &Value, cmd: &str) -> Result<Option<String>> {
    match p.get("search") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let s = v.as_str().ok_or_else(|| bad(cmd, "`search` must be a string"))?;
            if s.chars().count() > MAX_SEARCH_CHARS {
                return Err(bad(cmd, format!("`search` is longer than {MAX_SEARCH_CHARS} characters")));
            }
            Ok(Some(s.to_lowercase()))
        }
    }
}

/// Add `packPath` to a wrapped command's result without assuming its shape.
fn with_pack_path(mut r: Value, rel: &str) -> Value {
    if let Value::Object(m) = &mut r {
        m.insert("packPath".into(), json!(rel));
    }
    r
}

/// The active document's id (a pack command's document is the one just made active).
fn active_doc(s: &Session) -> Result<photocraft_doc::DocId> {
    s.active().map(|st| st.doc.id).ok_or(EngineError::NoDocument)
}

/// Open a pack folder (`pack.open`): scan it for textures and make it the workspace.
fn open(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "pack.open";
    let path = p
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| bad(CMD, "pass `path`, the pack folder"))?;
    let (info, textures) = pack::scan(path).map_err(other)?;
    let mut categories = serde_json::Map::new();
    for t in &textures {
        let n = categories.get(&t.category).and_then(Value::as_u64).unwrap_or(0) + 1;
        categories.insert(t.category.clone(), json!(n));
    }
    let texture_count = textures.len();
    let warnings = info.warnings.clone();
    let name = info.name.clone();
    let description = info.description.clone();
    let pack_format = info.pack_format;
    s.pack.pack = Some(info);
    s.pack.textures = textures;
    s.pack.targets.clear();
    Ok(json!({
        "path": path,
        "name": name,
        "description": description,
        "packFormat": pack_format,
        "textureCount": texture_count,
        "categories": Value::Object(categories),
        "warnings": warnings,
    }))
}

/// Close the pack workspace (`pack.close`).
fn close(s: &mut Session, _: &Value) -> Result<Value> {
    let had = s.pack.pack.is_some();
    s.pack.close();
    Ok(json!({"closed": had}))
}

/// The workspace's summary (`pack.info`), without rescanning.
fn info(s: &mut Session, _: &Value) -> Result<Value> {
    match &s.pack.pack {
        Some(info) => Ok(json!({
            "open": true,
            "path": info.root,
            "name": info.name,
            "description": info.description,
            "packFormat": info.pack_format,
            "textureCount": s.pack.textures.len(),
            "warnings": info.warnings,
        })),
        None => Ok(json!({"open": false})),
    }
}

/// List the pack's textures (`pack.list`), optionally filtered by category and a case-insensitive
/// search of the path.
fn list(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "pack.list";
    pack_open(s).map_err(other)?;
    let category = p.get("category").and_then(Value::as_str);
    let search = search_param(p, CMD)?;
    let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(500);
    if limit == 0 || limit > MAX_LIST_LIMIT {
        return Err(bad(CMD, format!("`limit` is 1..={MAX_LIST_LIMIT}")));
    }
    let offset = p.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let textures = &s.pack.textures;
    let filtered: Vec<&pack::PackTexture> = textures
        .iter()
        .filter(|t| category.is_none_or(|c| c == t.category))
        .filter(|t| search.as_deref().is_none_or(|q| t.rel.to_lowercase().contains(q)))
        .collect();
    let total = filtered.len() as u64;
    let start = offset.min(total) as usize;
    let textures: Vec<Value> = filtered
        .into_iter()
        .skip(start)
        .take(limit as usize)
        .map(|t| json!({"path": t.rel, "namespace": t.namespace, "category": t.category}))
        .collect();
    Ok(json!({"offset": offset, "total": total, "count": textures.len(), "textures": textures}))
}

/// The existence check `pack.openTexture` runs before importing. Unreachable on the web (the
/// command is disabled there first).
#[cfg(not(target_arch = "wasm32"))]
fn ensure_texture_file(s: &Session, rel: &str) -> Result<std::path::PathBuf> {
    const CMD: &str = "pack.openTexture";
    let path = s.pack.abs(rel).ok_or_else(|| bad(CMD, "open a pack first (`pack.open`)"))?;
    let meta = std::fs::metadata(&path).map_err(|e| other(format!("`{rel}` could not be opened: {e}")))?;
    if !meta.is_file() {
        return Err(other(format!("`{rel}` is not a file")));
    }
    if meta.len() > MAX_TEXTURE_FILE_BYTES {
        return Err(other(format!("`{rel}` is larger than the texture limit ({} bytes)", MAX_TEXTURE_FILE_BYTES)));
    }
    Ok(path)
}

#[cfg(target_arch = "wasm32")]
fn ensure_texture_file(_s: &Session, _rel: &str) -> Result<std::path::PathBuf> {
    Err(other("not available on the web (no file system)"))
}

/// Open a pack texture as a document (`pack.openTexture`): the bytes are read from the pack and
/// imported like File › Open, and the document is bound to its pack path.
fn open_texture(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "pack.openTexture";
    let rel = rel_param(p, CMD)?;
    if !rel.ends_with(".png") {
        return Err(bad(CMD, "`path` is a texture: it ends in .png"));
    }
    if !s.pack.textures.iter().any(|t| t.rel == rel) {
        return Err(other(format!("`{rel}` is not a texture of the open pack (reopen the pack to pick up new files)")));
    }
    let path = ensure_texture_file(s, &rel)?;
    let name = rel.rsplit('/').next().unwrap_or(rel.as_str()).to_string();
    // Texture packs are sRGB PNGs: they import like any file, with the engine's import
    // diagnostics and memory gate.
    let bytes = crate::file_cmds::read_file_limited(path.to_string_lossy().as_ref(), MAX_TEXTURE_FILE_BYTES)
        .map_err(|e| other(format!("`{rel}` could not be read: {e}")))?;
    let r = crate::file_cmds::open_bytes_as(s, &name, &bytes, None, Some(path.to_string_lossy().into_owned()))?;
    if let Ok(doc) = active_doc(s) {
        s.pack.register_target(doc, &rel);
    }
    Ok(with_pack_path(r, &rel))
}

/// The parent-directory creation `pack.saveTexture` runs for a texture created in a new folder.
#[cfg(not(target_arch = "wasm32"))]
fn ensure_parent_dir(path: &std::path::Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| other(format!("`{}` could not be created: {e}", dir.display())))?;
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn ensure_parent_dir(_path: &std::path::Path) -> Result<()> {
    Err(other("not available on the web (no file system)"))
}

/// Create a new texture document in the pack (`pack.newTexture`): a transparent canvas at the
/// texture's size, named after the file, bound to its pack path so `pack.saveTexture` writes it
/// into the pack (its folder is created on save if needed).
fn new_texture(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "pack.newTexture";
    let rel = rel_param(p, CMD)?;
    pack::check_texture_rel(&rel).map_err(|e| bad(CMD, e))?;
    let width = side(p, "width", 16, CMD)?;
    let height = side(p, "height", width, CMD)?;
    let path = s.pack.abs(&rel).ok_or_else(|| bad(CMD, "open a pack first (`pack.open`)"))?;
    let name = rel.rsplit('/').next().unwrap_or(rel.as_str()).trim_end_matches(".png").to_string();
    let r = s.execute("file.new", json!({"width": width, "height": height, "background": "transparent", "name": name}))?;
    if let Ok(doc) = active_doc(s) {
        s.pack.register_target(doc, &rel);
    }
    let mut out = with_pack_path(r, &rel);
    if let Value::Object(m) = &mut out {
        m.insert("absolutePath".into(), json!(path.to_string_lossy()));
        m.insert("width".into(), json!(width));
        m.insert("height".into(), json!(height));
    }
    Ok(out)
}

/// Save the active document into the pack (`pack.saveTexture`): the bound pack path, or `path`.
/// The document keeps the file as its own (like a Save As), so File › Save updates the pack too.
fn save_texture(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "pack.saveTexture";
    let rel = match p.get("path").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) {
        Some(r) => {
            pack::check_rel(r).map_err(|e| bad(CMD, e))?;
            r.to_string()
        }
        None => {
            let doc = active_doc(s)?;
            s.pack
                .target_of(doc)
                .ok_or_else(|| bad(CMD, "the document is not a pack texture; pass `path` or create it with `pack.newTexture`"))?
                .to_string()
        }
    };
    let path = s.pack.abs(&rel).ok_or_else(|| bad(CMD, "open a pack first (`pack.open`)"))?;
    ensure_parent_dir(&path)?;
    let path = path.to_string_lossy().into_owned();
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = (*st.doc).clone();
    let warnings = crate::file_cmds::save_doc(&doc, &path, crate::file_cmds::SaveOpts::default())?;
    if let Some(st) = s.active_mut() {
        st.saved_to(path.clone());
        st.saved_revision = st.revision;
    }
    Ok(json!({"packPath": rel, "path": path, "warnings": warnings}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "pack.open",
            label: "Open Texture Pack Folder…",
            menu: &[],
            shortcut: None,
            params: r#"{"path":str (a folder)} → {path,name,description,packFormat,textureCount,categories,warnings} (a missing or unparsable pack.mcmeta is a warning, not an error)"#,
            enabled: has_filesystem,
            run: open,
            journal: false,
        },
        CommandSpec {
            id: "pack.close",
            label: "Close Texture Pack",
            menu: &[],
            shortcut: None,
            params: "{} → {closed}",
            enabled: always,
            run: close,
            journal: false,
        },
        CommandSpec {
            id: "pack.info",
            label: "Texture Pack Info",
            menu: &[],
            shortcut: None,
            params: "{} → {open,path,name,description,packFormat,textureCount,warnings}",
            enabled: always,
            run: info,
            journal: false,
        },
        CommandSpec {
            id: "pack.list",
            label: "List Pack Textures",
            menu: &[],
            shortcut: None,
            params: r#"{"category":str?,"search":str? (case-insensitive substring of the path),"limit":1..=5000=500,"offset":u64?=0} → {offset,total,count,textures:[{path,namespace,category}]}"#,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "pack.openTexture",
            label: "Open Pack Texture",
            menu: &[],
            shortcut: None,
            params: r#"{"path":str (a texture of the open pack)} → file.open's result plus {packPath} (the document is bound to the pack texture, so `pack.saveTexture` writes it back)"#,
            enabled: has_filesystem,
            run: open_texture,
            journal: false,
        },
        CommandSpec {
            id: "pack.newTexture",
            label: "New Pack Texture…",
            menu: &[],
            shortcut: None,
            params: r#"{"path":str (assets/<namespace>/textures/<category>/<name>.png),"width":1..=4096=16,"height":1..=4096 (default: width)} → file.new's result plus {packPath,absolutePath,width,height}"#,
            enabled: has_filesystem,
            run: new_texture,
            journal: false,
        },
        CommandSpec {
            id: "pack.saveTexture",
            label: "Save Texture into Pack",
            menu: &[],
            shortcut: None,
            params: r#"{"path":str? (the pack-relative path; default: the active document's bound pack texture)} → {packPath,path,warnings} (the folder is created if needed; the document keeps the file as its own)"#,
            enabled: has_filesystem,
            run: save_texture,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;

//! Window › Pack Explorer: the Minecraft Java Edition texture pack workspace
//! (`photocraft_engine::pack_cmds`).
//!
//! The panel shows the open pack's textures as a thumbnail grid, filtered by category and
//! searched by name; clicking a texture opens it as a document bound to its pack path, so
//! File › Save writes back into the pack. New textures are created straight into the pack's
//! layout (`pack.newTexture`) and saved with `pack.saveTexture`, which makes the folders the
//! texture needs.
//!
//! Every state change here dispatches an engine command (`pack.*`), like every panel: the
//! control channel, CLI and MCP drive the same workspace. Reading the session's pack state for
//! display changes nothing and needs no command.

use egui::{Color32, Sense, TextureHandle, TextureOptions, Vec2};
use serde_json::json;

use crate::theme::Tokens;
use crate::{PhotocraftApp, widgets};

/// One grid cell: the thumbnail square plus its file name.
const CELL: f32 = 64.0;
/// The thumbnail square inside a cell.
const THUMB: f32 = 48.0;
/// The checkerboard cell behind a thumbnail.
const CHECKER: f32 = 8.0;
/// The most textures drawn at once, so a huge pack cannot stall a frame; the search narrows the
/// rest (pack.open's own scan cap is far higher).
const MAX_CELLS: usize = 2_000;

pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(pack) = app.session.pack.pack.clone() else {
        empty_state(app, ui, &t);
        return;
    };

    // Header: name, pack format and description.
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(&pack.name).size(13.0).strong());
        if let Some(format) = pack.pack_format {
            ui.label(
                egui::RichText::new(crate::i18n::fmt(tl!("format {n}"), &[("n", &format.to_string())]))
                    .color(t.text_faint)
                    .size(11.0),
            );
        }
        if ui.add(egui::Button::new(tl!("Rescan"))).clicked() {
            let path = pack.root.clone();
            let _ = app.run("pack.open", json!({"path": path}));
        }
    });
    if !pack.description.is_empty() {
        ui.add_space(2.0);
        ui.label(egui::RichText::new(&pack.description).color(t.text_faint).size(11.5).weak());
    }
    for warning in &pack.warnings {
        ui.label(egui::RichText::new(warning).color(t.text_faint).size(11.0).weak());
    }
    ui.add_space(4.0);
    ui.separator();

    toolbar(app, ui);

    // Search and category filter.
    let categories: Vec<String> = {
        let mut seen: Vec<String> = Vec::new();
        for tex in &app.session.pack.textures {
            if !seen.contains(&tex.category) {
                seen.push(tex.category.clone());
            }
        }
        seen
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let search = &mut app.ui.pack_panel.search;
        let width = (ui.available_width() - 150.0).max(60.0);
        let field = egui::TextEdit::singleline(search).hint_text(tl!("Search textures")).desired_width(width);
        ui.add(field);
        craft_ui::ComboBox::from_id_salt("pack-category")
            .selected_text(category_label(app.ui.pack_panel.category.as_deref()))
            .width(100.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut app.ui.pack_panel.category, None, tl!("All"));
                for c in &categories {
                    ui.selectable_value(&mut app.ui.pack_panel.category, Some(c.clone()), c.clone());
                }
            });
    });

    grid(app, ui);
}

/// No pack open: a short explanation and the way in.
fn empty_state(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(tl!("Open a Minecraft texture pack folder to browse and edit its textures."))
                .color(t.text_faint)
                .size(11.5),
        );
        ui.add_space(6.0);
        if ui.add(egui::Button::new(tl!("Open Pack Folder…"))).clicked() {
            ask_open_pack(app);
        }
    });
}

/// The toolbar: open another pack, create a texture, save back into the pack, close.
fn toolbar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let new_texture = app.ui.pack_panel.new_texture;
    let bound = active_pack_texture(app).is_some();
    ui.horizontal(|ui| {
        if ui.add(egui::Button::new(tl!("Open Pack Folder…"))).clicked() {
            ask_open_pack(app);
        }
        if ui.add(egui::Button::new(if new_texture { tl!("Cancel New Texture") } else { tl!("New Texture…") })).clicked() {
            let st = &mut app.ui.pack_panel;
            st.new_texture = !st.new_texture;
            if st.new_texture {
                // A fresh form starts from the filter's category (or block) with a 16×16 size.
                let category = st.category.clone().unwrap_or_else(|| "block".into());
                let name = st.new_name().unwrap_or_else(|| "texture".into());
                st.new_path = format!("assets/minecraft/textures/{category}/{name}.png");
                st.new_width = 16.0;
                st.new_height = 16.0;
            }
        }
        if ui.add_enabled(bound, egui::Button::new(tl!("Save into Pack"))).clicked() {
            let _ = app.run("pack.saveTexture", json!({}));
        }
        if ui.add(egui::Button::new(tl!("Close Pack"))).clicked() {
            let _ = app.run("pack.close", json!({}));
        }
    });
    if new_texture {
        new_texture_form(app, ui);
    }
}

/// The inline New Texture form: a pack path and a canvas size; Create dispatches
/// `pack.newTexture`, which makes the document and binds it so Save into Pack writes the file.
fn new_texture_form(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    egui::Frame::group(ui.style()).inner_margin(6.0).show(ui, |ui| {
        let path = app.ui.pack_panel.new_path.trim().to_string();
        let valid = photocraft_engine::pack::check_texture_rel(&path).is_ok();
        let width = (ui.available_width() - 210.0).max(120.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Path:")).size(11.5));
            ui.add(
                egui::TextEdit::singleline(&mut app.ui.pack_panel.new_path)
                    .hint_text(tl!("assets/minecraft/textures/block/stone.png"))
                    .desired_width(width),
            );
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Size:")).size(11.5));
            let drag = |ui: &mut egui::Ui, v: &mut f64| {
                ui.add(
                    egui::DragValue::new(v)
                        .speed(1.0)
                        .range(1.0..=4096.0)
                        .custom_formatter(|n, _| format!("{}", n.round() as i64)),
                );
            };
            drag(ui, &mut app.ui.pack_panel.new_width);
            ui.label("×");
            drag(ui, &mut app.ui.pack_panel.new_height);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(valid, egui::Button::new(tl!("Create"))).clicked() {
                    let st = &app.ui.pack_panel;
                    let params = json!({"path": path, "width": st.new_width.round() as i64, "height": st.new_height.round() as i64});
                    let _ = app.run("pack.newTexture", params);
                    app.ui.pack_panel.new_texture = false;
                }
            });
        });
    });
}

/// Ask for the pack folder with the shell's folder dialog, then open it.
fn ask_open_pack(app: &mut PhotocraftApp) {
    let _ = app.ask_folder(|app, path| {
        if path.trim().is_empty() {
            return Ok(json!(null));
        }
        app.run("pack.open", json!({"path": path}))
    });
}

/// The active document's bound pack texture, if the active document is a pack texture.
fn active_pack_texture(app: &PhotocraftApp) -> Option<String> {
    let doc = app.session.active()?.doc.id;
    app.session.pack.target_of(doc).map(str::to_string)
}

/// The category filter's selected label (`All` when unfiltered).
fn category_label(category: Option<&str>) -> String {
    category.map(str::to_string).unwrap_or_else(|| tl!("All").into())
}

/// The texture grid: clickable thumbnails, the active document's texture highlighted.
fn grid(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let search = app.ui.pack_panel.search.to_lowercase();
    let category = app.ui.pack_panel.category.clone();
    let textures: Vec<(String, String, String)> = app
        .session
        .pack
        .textures
        .iter()
        .filter(|tex| category.as_deref().is_none_or(|c| c == tex.category))
        .filter(|tex| search.is_empty() || tex.rel.to_lowercase().contains(&search))
        .take(MAX_CELLS)
        .map(|tex| (tex.rel.clone(), tex.namespace.clone(), tex.category.clone()))
        .collect();
    if textures.is_empty() {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tl!("No textures match the search.")).color(t.text_faint).size(11.5));
        return;
    }
    let active_rel = active_pack_texture(app);
    let root = app.session.pack.pack.as_ref().map(|p| p.root.clone()).unwrap_or_default();
    let ctx = ui.ctx().clone();
    let mut open_texture: Option<String> = None;
    egui::ScrollArea::vertical().id_salt("pack-textures").auto_shrink([false, true]).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 8.0);
            for (rel, namespace, cat) in &textures {
                let selected = active_rel.as_deref() == Some(rel.as_str());
                let tooltip = format!("{rel}\n{namespace} · {cat}");
                if cell(&ctx, &root, rel, selected, &tooltip, ui) {
                    open_texture = Some(rel.clone());
                }
            }
        });
    });
    if let Some(rel) = open_texture {
        let _ = app.run("pack.openTexture", json!({"path": rel}));
    }
}

/// Draw one cell; returns whether it was clicked. The thumbnail is decoded once per file
/// revision and cached on the context.
fn cell(ctx: &egui::Context, root: &str, rel: &str, selected: bool, tooltip: &str, ui: &mut egui::Ui) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(CELL), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, rel));
    let inner = egui::Rect::from_center_size(rect.center(), Vec2::splat(THUMB));
    match thumbnail(ctx, root, rel) {
        Some(handle) => {
            // A checkerboard behind the texture shows its transparency.
            widgets::checker(ui.painter(), inner, CHECKER);
            ui.painter().image(
                handle.id(),
                inner,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            // Undecodable or unreadable: a dimmed placeholder.
            ui.painter().rect_filled(inner, 2.0, t.field);
            ui.painter().text(inner.center(), egui::Align2::CENTER_CENTER, "PNG?", egui::FontId::proportional(10.0), t.text_faint);
        }
    }
    let stroke = if selected { egui::Stroke::new(2.0, t.accent) } else { egui::Stroke::new(1.0, t.field_border) };
    ui.painter().rect_stroke(rect.shrink(2.0), 3.0, stroke, egui::StrokeKind::Outside);
    resp.on_hover_text(tooltip).clicked()
}

/// The thumbnail for a texture, decoded once per file revision (cached on the context).
/// File-system thumbnails are desktop-only; the web build shows the placeholder.
#[cfg(not(target_arch = "wasm32"))]
fn thumbnail(ctx: &egui::Context, root: &str, rel: &str) -> Option<TextureHandle> {
    use std::path::Path;
    if root.is_empty() {
        return None;
    }
    let path = Path::new(root).join(rel);
    let path = path.to_string_lossy().into_owned();
    let meta = std::fs::metadata(&path).ok()?;
    let key = (rel.to_string(), meta.len(), meta.modified().ok());
    let id = egui::Id::unique(("pack-thumb", &key));
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(tex);
    }
    let image = load_thumbnail(&path, meta.len())?;
    let tex = ctx.load_texture(format!("pack {rel}"), image, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}

/// The web build has no file system to read textures from.
#[cfg(target_arch = "wasm32")]
fn thumbnail(_ctx: &egui::Context, _root: &str, _rel: &str) -> Option<TextureHandle> {
    None
}

/// Decode a texture file into a thumbnail image (bomb-guarded, like the recent files' ones).
#[cfg(not(target_arch = "wasm32"))]
fn load_thumbnail(path: &str, file_len: u64) -> Option<egui::ColorImage> {
    if file_len > 64 << 20 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let opts = photocraft_codecs::DecodeOptions {
        limits: photocraft_codecs::Limits { max_width: 4096, max_height: 4096, max_pixels: 4096 * 4096, max_alloc: 4096 * 4096 * 4 },
        ..Default::default()
    };
    let img = photocraft_codecs::decode_with(&bytes, &opts).ok()?;
    let img = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
    if img.data().len() != img.width() as usize * img.height() as usize * 4 {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.data()))
}

/// The New Texture form's default file name from the path field (`stone.png`'s stem).
impl crate::state::PackPanelState {
    fn new_name(&self) -> Option<String> {
        let name = self.new_path.rsplit('/').next().unwrap_or_default().trim();
        (!name.is_empty()).then(|| name.trim_end_matches(".png").to_string())
    }
}

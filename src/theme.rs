//! Catppuccin Mocha theme + phosphor icons + minor visual tweaks.

use eframe::egui;

pub fn apply(ctx: &egui::Context) {
    catppuccin_egui::set_theme(ctx, catppuccin_egui::MOCHA);

    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    ctx.style_mut(|style| {
        let visuals = &mut style.visuals;
        visuals.window_rounding = egui::Rounding::same(10.0);
        visuals.menu_rounding = egui::Rounding::same(8.0);
        for ws in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            ws.rounding = egui::Rounding::same(6.0);
        }
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
    });
}

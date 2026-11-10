//! What the panel sets of a Campaign Evolved preview, and what a load reads.

use super::super::ModelPreviewState;
use super::super::load::OwnViewSettings;
use crate::app::ui_kit::checkbox::CheckboxUi as _;

/// The panel's Campaign Evolved settings, and what the cached preview was
/// built with.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct CampaignEvolvedState {
    /// Decode the full-resolution **Nanite** geometry for static mesh pieces
    /// instead of the coarse LOD fallback. On by default for faithful
    /// previews; users can disable it for unusually heavy models.
    pub(in crate::app) high_detail: bool,
    /// The `high_detail` value the cached `data` was built with, so toggling
    /// it invalidates the cache and reloads.
    pub(in crate::app) loaded_high_detail: bool,
}

impl Default for CampaignEvolvedState {
    fn default() -> Self {
        Self {
            high_detail: true,
            loaded_high_detail: false,
        }
    }
}

impl CampaignEvolvedState {
    /// Whether the cached preview was built with what is set now.
    pub(in crate::app) fn loaded_as_set(&self) -> bool {
        self.loaded_high_detail == self.high_detail
    }
}

/// The load-affecting slice of [`CampaignEvolvedState`].
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::app) struct CampaignEvolvedLoad {
    pub(in crate::app) high_detail: bool,
}

impl CampaignEvolvedLoad {
    pub(in crate::app) fn of(state: &CampaignEvolvedState) -> Self {
        Self {
            high_detail: state.high_detail,
        }
    }

    /// Note on `state` that its preview is being built with these.
    pub(in crate::app) fn loading_into(self, state: &mut CampaignEvolvedState) {
        state.loaded_high_detail = self.high_detail;
    }
}

/// The view menu's High Detail switch, in place of the tag overlays' toggles,
/// which a model drawn from its Unreal meshes has none of.
pub(in crate::app::model_preview) const VIEW_SETTINGS: OwnViewSettings = OwnViewSettings {
    draw: draw_view_settings,
    hides_overlays: true,
};

fn draw_view_settings(ui: &mut eframe::egui::Ui, state: &mut ModelPreviewState) {
    ui.baboon_checkbox(&mut state.campaign_evolved.high_detail, "High Detail")
        .on_hover_text(
            "Decode full-resolution Nanite geometry instead of Unreal's coarse fallback.",
        );
}

#[cfg(test)]
mod tests {
    use super::super::super::ModelPreviewState;

    #[test]
    fn campaign_evolved_preview_defaults_to_full_detail() {
        assert!(ModelPreviewState::default().campaign_evolved.high_detail);
    }
}

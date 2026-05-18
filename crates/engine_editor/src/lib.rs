use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditorMode {
    Edit,
    Play,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorPanel {
    pub id: String,
    pub title: String,
}

impl EditorPanel {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorShell {
    pub mode: EditorMode,
    pub panels: Vec<EditorPanel>,
}

impl EditorShell {
    pub fn full_editor_defaults() -> Self {
        Self {
            mode: EditorMode::Edit,
            panels: vec![
                EditorPanel::new("scene_hierarchy", "Scene Hierarchy"),
                EditorPanel::new("inspector", "Inspector"),
                EditorPanel::new("asset_browser", "Asset Browser"),
                EditorPanel::new("viewport", "Viewport"),
                EditorPanel::new("console", "Console"),
            ],
        }
    }

    pub fn enter_play_mode(&mut self) {
        self.mode = EditorMode::Play;
    }

    pub fn enter_edit_mode(&mut self) {
        self.mode = EditorMode::Edit;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_editor_contains_required_panels() {
        let editor = EditorShell::full_editor_defaults();
        let panel_ids = editor
            .panels
            .iter()
            .map(|panel| panel.id.as_str())
            .collect::<Vec<_>>();

        assert!(panel_ids.contains(&"scene_hierarchy"));
        assert!(panel_ids.contains(&"inspector"));
        assert!(panel_ids.contains(&"asset_browser"));
        assert!(panel_ids.contains(&"viewport"));
    }
}

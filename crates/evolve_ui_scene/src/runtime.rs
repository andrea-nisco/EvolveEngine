//! The UI of a game as one thing that can be drawn: the screens, the text system and the scene they become.

use crate::source::UiSource;
use crate::UiLayer;
use evolve_engine::scene::Scene;
use evolve_ui_core::{ImageId, ScreenMetrics, ScreenStack};
use evolve_ui_text::TextSystem;

pub struct UiRuntime {
    pub stack: ScreenStack,
    pub text: TextSystem,
    layer: UiLayer,
}

impl Default for UiRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl UiRuntime {
    pub fn new() -> Self {
        Self { stack: ScreenStack::new(), text: TextSystem::new(), layer: UiLayer::new() }
    }

    /// Makes an SVG known; returns its id and the size of the drawing (for a tree's `add_image`).
    pub fn add_svg(&mut self, data: &[u8]) -> Result<(ImageId, (f32, f32)), String> {
        let id = self.layer.add_svg(data)?;
        let size = self.layer.image_size(id).ok_or("the image has no size")?;
        Ok((id, size))
    }

    pub fn layer(&self) -> &UiLayer {
        &self.layer
    }
}

impl UiSource for UiRuntime {
    fn frame(&mut self, screen: &ScreenMetrics) -> &Scene {
        let (list, made) = self.stack.draw_list(screen, &mut self.text);
        if made {
            self.layer.update(list, &mut self.text);
        }
        self.layer.scene()
    }
}

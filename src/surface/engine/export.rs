use super::Engine;
use crate::surface::{Host, Overlay, Rect};

impl<H: Host, O: Overlay> Engine<H, O> {
    /// With `GRAMLY_STATE=<file>`, what is on screen is written there whenever
    /// it changes: the text, each fix, where its underline is drawn, and the
    /// card's rows. `gramly lab --e2e` reads it to point the mouse at a real
    /// underline and click a real button. Rectangles are in global display
    /// coordinates, top-left origin, as mouse events take them.
    pub(super) fn export(&mut self) {
        let Some(path) = std::env::var_os("GRAMLY_STATE") else {
            return;
        };
        let rect = |r: Rect| [r.x, r.y, r.w, r.h];
        let fixes: Vec<serde_json::Value> = self
            .fixes
            .iter()
            .enumerate()
            .map(|(i, fix)| {
                serde_json::json!({
                    "start": fix.start, "end": fix.end,
                    "original": fix.original(&self.text), "replacement": fix.replacement,
                    "drawn": self.words.iter().find(|(w, _)| *w == i).map(|(_, r)| rect(*r)),
                })
            })
            .collect();
        let rows: Vec<serde_json::Value> = self
            .overlay
            .card_rows()
            .into_iter()
            .map(|(action, r)| serde_json::json!({"action": action.name(), "rect": rect(r)}))
            .collect();
        let snapshot = serde_json::json!({
            "text": self.text,
            "field": self.field.is_some(),
            "ai_current": self.ai_current,
            "fixes": fixes,
            "card": self.card_for,
            "rows": rows,
        })
        .to_string();
        if self.exported.as_deref() != Some(snapshot.as_str()) {
            let _ = std::fs::write(&path, &snapshot);
            self.exported = Some(snapshot);
        }
    }
}

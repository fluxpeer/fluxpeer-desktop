// SPDX-License-Identifier: AGPL-3.0-or-later
//! Quit requires an explicit successful teardown ack or the user's override.
#[derive(Default, Debug, PartialEq)]
pub enum QuitState { #[default] Idle, Pending, Failed(String) }
impl QuitState {
    pub fn begin(&mut self) -> bool {
        if matches!(self, Self::Pending) { return false; }
        *self = Self::Pending; true
    }
    pub fn complete(&mut self, result: Result<(), String>) -> bool {
        if !matches!(self, Self::Pending) { return false; }
        match result {
            Ok(()) => true,
            Err(error) => { *self = Self::Failed(error); false },
        }
    }
    pub fn can_force(&self) -> bool { matches!(self, Self::Failed(_)) }
}
pub fn check_ack(value: serde_json::Value) -> Result<(), String> {
    if let Some(error) = value.get("error").and_then(|v| v.as_str()) { return Err(error.into()); }
    if value["ok"] == true && value["shutdown"] == true { Ok(()) }
    else { Err("daemon did not confirm disconnection".into()) }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn success_requires_explicit_teardown_ack() {
        for value in [json!({}), json!({"ok":true}), json!({"shutdown":true}), json!({"ok":true,"shutdown":false}), json!({"ok":true,"shutdown":true,"error":"cleanup failed"})] {
            let mut state = QuitState::default(); assert!(state.begin());
            assert!(!state.complete(check_ack(value))); assert!(state.can_force());
        }
        let mut state = QuitState::default(); assert!(state.begin());
        assert!(state.complete(check_ack(json!({"ok":true,"shutdown":true}))));
    }
    #[test]
    fn failure_stays_open_cancel_resets_and_duplicate_quit_is_suppressed() {
        let mut state = QuitState::default(); assert!(!state.can_force());
        assert!(state.begin()); assert!(!state.begin()); assert!(!state.can_force());
        assert!(!state.complete(Err("connection refused".into())));
        assert_eq!(state, QuitState::Failed("connection refused".into())); assert!(state.can_force());
        state = QuitState::Idle;
        assert!(!state.can_force()); assert!(!state.complete(Ok(())));
        assert!(state.begin()); assert!(state.complete(Ok(())));
    }
}

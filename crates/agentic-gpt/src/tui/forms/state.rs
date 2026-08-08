#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EditableListState {
    items: Vec<String>,
    focus: usize,
}

impl EditableListState {
    pub(crate) fn new(items: Vec<String>) -> Self {
        let mut state = Self { items, focus: 0 };
        state.normalize_focus();
        state
    }

    pub(crate) fn items(&self) -> &[String] {
        &self.items
    }

    pub(crate) fn into_items(self) -> Vec<String> {
        self.items
    }

    pub(crate) fn focus(&self) -> usize {
        self.focus
    }

    pub(crate) fn set_focus(&mut self, index: usize) {
        self.focus = index;
        self.normalize_focus();
    }

    pub(crate) fn focused(&self) -> Option<&str> {
        self.items.get(self.focus).map(String::as_str)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(crate) fn move_next(&mut self) {
        if self.focus + 1 < self.items.len() {
            self.focus += 1;
        }
    }

    pub(crate) fn move_previous(&mut self) {
        self.focus = self.focus.saturating_sub(1);
    }

    pub(crate) fn add_after_focused(&mut self) -> usize {
        let index = if self.items.is_empty() {
            0
        } else {
            (self.focus + 1).min(self.items.len())
        };
        self.items.insert(index, String::new());
        self.focus = index;
        index
    }

    pub(crate) fn set_focused(&mut self, value: impl Into<String>) -> bool {
        let Some(item) = self.items.get_mut(self.focus) else {
            return false;
        };
        *item = value.into();
        true
    }

    pub(crate) fn delete_focused(&mut self) -> Option<String> {
        if self.items.is_empty() {
            return None;
        }
        let removed = self.items.remove(self.focus);
        self.normalize_focus();
        Some(removed)
    }

    fn normalize_focus(&mut self) {
        self.focus = match self.items.len() {
            0 => 0,
            len => self.focus.min(len - 1),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_state_adds_edits_deletes_and_keeps_focus_valid() {
        let mut state = EditableListState::new(vec!["/one".into(), "/two".into()]);
        assert_eq!(state.focused(), Some("/one"));

        state.set_focus(99);
        assert_eq!(state.focus(), 1);
        assert_eq!(state.focused(), Some("/two"));

        let added = state.add_after_focused();
        assert_eq!(added, 2);
        assert_eq!(state.focus(), 2);
        assert_eq!(state.focused(), Some(""));
        assert!(state.set_focused("/three"));
        assert_eq!(state.focused(), Some("/three"));

        assert_eq!(state.delete_focused().as_deref(), Some("/three"));
        assert_eq!(state.focus(), 1);
        assert_eq!(state.focused(), Some("/two"));
    }

    #[test]
    fn empty_list_can_add_first_item_and_delete_back_to_empty() {
        let mut state = EditableListState::default();
        assert!(state.is_empty());
        assert_eq!(state.add_after_focused(), 0);
        assert!(state.set_focused("/root"));
        assert_eq!(state.items(), &["/root"]);
        assert_eq!(state.delete_focused().as_deref(), Some("/root"));
        assert!(state.is_empty());
        assert_eq!(state.focus(), 0);
    }
}

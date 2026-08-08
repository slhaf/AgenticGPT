use std::fmt;

use crossterm::event::KeyCode;

use crate::config_setup::SetupField;

pub(crate) struct EditState {
    pub(crate) field: SetupField,
    pub(crate) buffer: String,
    pub(crate) cursor: usize,
}

impl EditState {
    pub(crate) fn new(field: SetupField, value: impl Into<String>) -> Self {
        let buffer = value.into();
        let cursor = buffer.chars().count();
        Self {
            field,
            buffer,
            cursor,
        }
    }
}

impl fmt::Debug for EditState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EditState")
            .field("field", &self.field)
            .field("buffer", &"[REDACTED]")
            .field("cursor", &self.cursor)
            .finish()
    }
}

pub(crate) fn apply_text_key(mut edit: EditState, key: KeyCode) -> EditState {
    match key {
        KeyCode::Char(character) => insert_at_cursor(&mut edit, character),
        KeyCode::Backspace => backspace(&mut edit),
        KeyCode::Delete => delete_at_cursor(&mut edit),
        KeyCode::Left => {
            edit.cursor = edit.cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            edit.cursor = (edit.cursor + 1).min(edit.buffer.chars().count());
        }
        KeyCode::Home => edit.cursor = 0,
        KeyCode::End => edit.cursor = edit.buffer.chars().count(),
        _ => {}
    }
    edit
}

fn insert_at_cursor(edit: &mut EditState, character: char) {
    let byte_index = byte_index_at_cursor(&edit.buffer, edit.cursor);
    edit.buffer.insert(byte_index, character);
    edit.cursor += 1;
}

fn backspace(edit: &mut EditState) {
    if edit.cursor == 0 {
        return;
    }
    let end = byte_index_at_cursor(&edit.buffer, edit.cursor);
    let start = byte_index_at_cursor(&edit.buffer, edit.cursor - 1);
    edit.buffer.replace_range(start..end, "");
    edit.cursor -= 1;
}

fn delete_at_cursor(edit: &mut EditState) {
    let start = byte_index_at_cursor(&edit.buffer, edit.cursor);
    let end = byte_index_at_cursor(&edit.buffer, edit.cursor + 1);
    if start < end {
        edit.buffer.replace_range(start..end, "");
    }
}

fn byte_index_at_cursor(value: &str, cursor: usize) -> usize {
    value
        .char_indices()
        .nth(cursor)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}

#[cfg(test)]
mod tests {
    use crate::config_setup::SetupField;

    use super::*;

    #[test]
    fn edit_state_starts_at_end_and_character_keys_only_change_ui_buffer() {
        let edit = EditState::new(SetupField::TunnelId, "confirmed-tunnel");
        assert_eq!(edit.cursor, "confirmed-tunnel".len());
        assert_eq!(edit.buffer, "confirmed-tunnel");
        assert_eq!(
            apply_text_key(edit, crossterm::event::KeyCode::Char('x')).buffer,
            "confirmed-tunnelx"
        );
    }
}

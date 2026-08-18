// Portions derived from OpenAI Codex apply-patch (Apache-2.0).
mod file_update;
mod parser;
mod seek_sequence;
mod streaming_parser;
mod text_file;
pub use file_update::apply_update;
pub use parser::{Hunk, ParseError, UpdateFileChunk, parse_patch};
use thiserror::Error;
#[derive(Debug, PartialEq)]
pub struct ApplyPatchArgs {
    pub patch: String,
    pub hunks: Vec<Hunk>,
    pub workdir: Option<String>,
    pub environment_id: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ApplyPatchFileUpdateMode {
    #[default]
    NormalizeToLf,
    PreserveLineEndings,
}
#[derive(Debug, Error, PartialEq)]
pub enum ApplyPatchError {
    #[error("{0}")]
    ComputeReplacements(String),
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_patch() {
        let p=parse_patch("*** Begin Patch\n*** Add File: add.txt\n+hello\n*** Delete File: delete.txt\n*** Update File: old.txt\n*** Move to: new.txt\n@@\n-old\n+new\n*** End Patch").unwrap();
        assert_eq!(p.hunks.len(), 3);
    }
    #[test]
    fn applies_update() {
        let p = parse_patch(
            "*** Begin Patch\n*** Update File: sample.txt\n@@\n-alpha\n+beta\n*** End Patch",
        )
        .unwrap();
        let Hunk::UpdateFile { chunks, .. } = &p.hunks[0] else {
            panic!()
        };
        assert_eq!(
            apply_update(
                "alpha\n",
                "sample.txt",
                chunks,
                ApplyPatchFileUpdateMode::PreserveLineEndings
            )
            .unwrap(),
            "beta\n"
        );
    }
}

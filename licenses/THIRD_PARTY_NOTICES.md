# 第三方组件声明

Linux 发行包中的 `agentic-gpt` 包含以下组件及字体。对应许可证全文位于 `licenses/`：

- **Obscura**：Apache License 2.0，源码位于 `vendor/obscura`。
- **V8**：V8 项目 BSD 3-Clause；Rust 绑定 `rusty_v8` 为 MIT。此发行版通过 Obscura 的 `deno_core` 使用 V8。许可证全文为 `LICENSE-V8.txt` 和 `LICENSE-rusty_v8.txt`。
- **Liberation 字体**（Liberation Sans、Serif、Mono）：SIL Open Font License 1.1。
- **DejaVu Sans**：Bitstream Vera 字体许可证；DejaVu 修改部分属于公有领域。许可证全文在 `LICENSE-DejaVu.txt`。
- **Noto Color Emoji**：SIL Open Font License 1.1。
- **cosmic-text**：MIT 或 Apache License 2.0。
- **Taffy**：MIT。
- **wreq-util**：Apache License 2.0。

字体嵌入于 Obscura 的渲染库。上述字体的版权和来源说明也见 `vendor/obscura/crates/obscura-render/assets/FONT-PROVENANCE.md`。

# Ambery 文档 i18n

`docs/`、`concepts.md`、`spec.md`、`docs-spec.md`、`CONTRIBUTING.md` 与 `README` 的双语配对规则。

- `foo.md` 是英文文档；`foo.zh.md` 是中文文档。两侧同权。
- 每个文件只使用一种语言的散文，不混排段落。机器契约字符串、路径、命令与代码块语言中立。
- 使用 `glossary.md` 的固定术语表。
- 改动任一侧后同步另一侧，并运行 `node scripts/verify-i18n-pairs.mjs`。

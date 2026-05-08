# notes/

`docs/handoff.md` の補助資料置き場。Phase ごとに進む実装の前提を集約する。

| ファイル | 内容 |
|---|---|
| `phase0.md` | Phase 0 の format reading 結果。HSDLib/csx を読み下した一次解釈。Phase 1 以降のすべての実装はここの解釈を ground truth として参照 |
| `phase4.md` | Phase 4 の parity harness 構成・落とし穴 (RGB5A3 の HSDLib 変数命名と byte order)・PyO3 surface |

参照元 path:

- HSDLib (Ploaj/HSDLib): `~/src/github.com/Ploaj/HSDLib/HSDRaw/` — format 正規 reference
- csx golden: `~/src/github.com/naari3/mkgp2-patch/tools/hsd/hsd_export_for_blender.csx`
- Blender consumer: `~/src/github.com/naari3/mkgp2-patch/tools/blender/blender_import_hsd.py`
- mkgp2docs (handoff の `mkgp2docs/` 参照は実際にはこちら):
  `~/src/github.com/dolphin-emu/dolphin/mkgp2docs/`
  - `hsd_to_blender_visual_pipeline.md` — 罠 6 件
  - `hsd_alias_and_blender_pipeline.md` — alias root の HSDLib API 動作確認
  - `hsd_parsing_tools.md` — HSDLib accessor 一覧
  - `mkgp2_course_layout_system.md` — コース構成
- lessons.md (channel-order の罠初出): `~/src/github.com/naari3/mkgp2-patch/tasks/lessons.md`
- vanilla corpus (round-trip ground truth, repo 外):
  `C:\Users\naari\Documents\Dolphin ROMs\Triforce\mkgp2\files\`

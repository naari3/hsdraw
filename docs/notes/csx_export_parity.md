# csx export parity

`crates/hsdraw-core/src/export.rs` の出す scene JSON は、開発初期に
HSDLib + `dotnet-script` で書かれた既存 csx (cross-platform script)
golden の出力と semantic-equal になるよう field を選んだ。本 doc は
その時の対応関係を記録する (= library 側の機能定義は `export.rs` の
DTO 構造体が一次資料、本 doc は **由来の補足**)。

## 由来 csx

`mkgp2-patch` リポジトリ (Blender add-on の母艦) の
`tools/hsd/hsd_export_for_blender.csx` が一次 reference。MKGP2 専用
ゲームではあるが、HSD-format generic なフィールド (joint hierarchy,
TRS, material colors, texture refs) しか抜いていないため、出力 JSON
schema 自体は HSDLib `HSDRawFile` 全般に適用可能と判断、本 library
の export 経路として採用した。

## 対応表 (csx 用語 → hsdraw DTO)

| csx 出力フィールド | export.rs DTO | 備考 |
|---|---|---|
| `scene.source_dat` | `Scene::source_dat` | 入力 .dat ファイル名 |
| `scene.tex_dir` | `Scene::tex_dir` | PNG dump 用 dir 名 |
| `scene.textures[]` | `Scene::textures` (`TextureDto`) | TObj + Image を flatten |
| `scene.materials[]` | `Scene::materials` (`MaterialDto`) | MObj + Material を flatten |
| `scene.joints[]` | `Scene::joints` (`JointDto`) | JObj DFS walk、`jobj_N` ID |
| `scene.joint_aliases` | `Scene::joint_aliases` | alias root → `jobj_N` map |
| `scene.meshes[]` | `Scene::meshes` (`MeshDto`) | DObj/POBJ → 頂点 + 三角形 |

詳細な field 順 / 命名 / 浮動小数 epsilon 等は parity テストが pin
している (`tests/parity*.rs`)。

## parity テストの位置づけ

- env-free `tests/parity_env_free.rs` (4 tests): hsdraw 単体での
  schema sanity check
- env-bound `tests/parity.rs` (要 corpus + csx 実行環境):
  - `HSDRAW_PARITY_CORPUS_DIR` (実 .dat ファイル群)
  - `HSDRAW_PARITY_CSX_DIR` (上記 csx を含むツリー)

env-bound パスは "外部 corpus との semantic 差を毎 commit で gate"
が目的。corpus の中身 (どのゲーム / どの bundle) は library の責務
ではなく、CI workflow / consumer 側で渡す事項。

## 現在の active corpus

開発時点で実際に validation 走らせている corpus は MKGP2 vanilla
6 course + 9-file writer round-trip (詳細は consumer 側の
project memory 参照)。これは **validation source** であって、library
が前提とする corpus ではない。新しい外部 consumer (別 HSD ゲーム /
別 bundle) を gate に追加する場合は同じ env var 経路で渡せる。

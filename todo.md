# todo

## Design philosophy (= 前提)

`hsdraw` は **HSD-format 汎用ライブラリ**であり、特定のゲーム
(MKGP2 / SSBM / Air Ride / 等) に縛られない設計を目標とする。

> **Open by default**: caller (Blender add-on, CLI ツール, 外部
> consumer 等) が HSD-format mechanic を read / write する際の素直な
> primitive を出すことに専念する。ある consumer の "観察" を library
> の preset / 制約 / 機能境界に焼き付けない。

現状: 開発過程で MKGP2 (mkgp2-patch addon) を validation corpus + 唯一
の active consumer として走らせてきたため、preset 値・機能境界・
docstring・test fixture が course/MKGP2 寄りに収束している。本 todo
は脱バイアスを段階的に進める project-level チェックリスト。

## Status legend

- 🔴 = 高優先 (= design philosophy 的に "嘘" になっている、または
  他 consumer 受け入れ時に阻害要因になる)
- 🟡 = 中優先 (= 構造的だが現状 caller の動作は阻害していない)
- ⚪ = 受け入れ要望が出てから着手で OK

---

## 1. 現状の MKGP2 / course-genre coupling — survey

### 1.1 直接 MKGP2 を名指ししている残骸

| 場所 | 内容 | 種別 |
|---|---|---|
| `crates/hsdraw-core/src/export.rs:3` | module doc が "Mirrors `mkgp2-patch/tools/hsd/hsd_export_for_blender.csx` exactly" でプロジェクト固有 path を直リンク | 🔴 |
| `crates/hsdraw-core/tests/parity.rs` | parity テストが `MKGP2_FILES_DIR` / `MKGP2_PATCH_DIR` env-bound、6 vanilla MKGP2 course を hard-code | 🟡 |
| `docs/handoff.md` | 元仕様。mkgp2-patch を consumer として書かれている | 🟡 |
| `docs/notes/phase{0,4,5,6,7}.md` | 開発フェーズ note。mkgp2-patch との突き合わせ前提 | 🟡 (履歴記録としては残してよい) |
| `docs/roadmap.md` 冒頭 | "Items are roughly ordered by leverage on the MKGP2 add-on workflow" | 🔴 |
| README `## What this is *not*` 節 | "vanilla MKGP2 corpus has zero hits across 7,812 textures so the addon can route paletted sources through RGB5A3 / RGB565 instead" — **ライブラリ機能制限を MKGP2 corpus データで正当化** | 🔴 |
| README test totals 節 | "9-file writer round-trip corpus" / "6 csx-parity courses" | 🟡 |

### 1.2 "course .dat" 前提の framing (= MKGP2 ではないが kart/course ジャンル決め打ち)

| 場所 | 内容 |
|---|---|
| `crates/hsdraw-core/src/writer.rs:28-29` | "n/a for course .dat" / "Roots[0] typed as MEX/kex disabling bufferAlign (n/a)" |
| README `## Status` テーブル | reader / writer 行 + `csx parity` 行が「course」前提 |
| `crates/hsdraw-core/src/dat.rs` `alloc_scene_data` | "scene_data" 名前 hard-code、refs=1 minimal (vanilla course SObj は refs=3 で COBJ/LObj 込み) |
| `MObj.allocate_unlit_color` / `MObj.allocate_textured` の field defaults | LIGHTMAP_DIFFUSE / MODULATE / REPEAT / ALPHA_MAT / blending=1.0 — course mesh の field pattern |

### 1.3 機能制限が MKGP2 corpus 観察に基づいて固定されている

| 場所 | 制限 | 寄り根拠 |
|---|---|---|
| `crates/hsdraw-core/src/gx_image.rs` encode 経路 | RGBA8 / RGB565 / RGB5A3 / CMP のみ、paletted 切り捨て | 「MKGP2 vanilla で paletted hit が無い」 |
| `crates/hsdraw-core/src/pobj_writer.rs` 固定 attribute format | POS F32×3 / NRM F32×3 / CLR0 RGBA8 / TEX0 F32×2 | "course mesh はこの format" 観察 |
| 同 single attribute-group per POBJ | 1 logical mesh = 1 group | course の各 POBJ は群 1 つ観察 |
| 同 65,535 verts cap | u16 vertex index | course mesh の頂点数を超えない前提 |
| `MeshBuilder` の greedy stripper | vertex-cache-aware ではない | course mesh のサイズなら誤差と判断 |

### 1.4 preset / docstring に固定された "vanilla 観察"

| API | 含まれる観察ベース |
|---|---|
| `MObj::allocate_textured` | render_flags = `CONSTANT \| DIFFUSE \| TEX0 \| ALPHA_MAT`、TObj に `LIGHTMAP_DIFFUSE` + `MODULATE` + `REPEAT` + `LINEAR` + `blending=1.0`。docstring に「the field values widely seen on textured POBJs across the HSD vanilla course corpus」— **observation source が MKGP2 1 ゲームしか無いのに「corpus」と一般化** |
| `MObj::allocate_unlit_color` | render_flags = `CONSTANT \| DIFFUSE`、shininess = 50.0 — fighter なら別 preset が欲しいはず |
| `Pobj` flags getter docstring | "0x8000 on statically-bound textured POBJs" — MKGP2 vanilla 94-97% を "real-world game corpora" と一般化 |
| `Dat::alloc_scene_data` | refs=1 minimal — fighter / character の SObj はそもそも形が違う |

### 1.5 テスト fixture / `dat_with_*` 構造

`tests/mobj_writer.rs` / `tests/pobj_writer.rs` の `dat_with_mobj` / `dat_with_pobj` が build する scaffolding は

```
scene_data root → SObj → JObjDescs[] → JObjDesc[0] → JObj (root_joint) → DObj → MObj / PObj
```

= **course `.dat` の全体構造そのまま**。fighter `.dat` は `SBM_FighterData` 直下に joint chain、character `.dat` は別の wrap → fixture が "HSD-format generic" じゃなくて "course-genre generic" になっている。

---

## 2. Action items — de-coupling

### 🔴 1. README §"What this is *not*" の paletted 制限 MKGP2 corpus 根拠を削除

現状: "vanilla MKGP2 corpus has zero hits across 7,812 textures so the addon can route paletted sources through RGB5A3 / RGB565 instead"

修正案: 「paletted format encoding is on the roadmap; for unpaletted formats use RGBA8 / RGB565 / RGB5A3 / CMP」程度の純機能的記述に。

### 🔴 2. `crates/hsdraw-core/src/export.rs:3` module doc の mkgp2-patch path 参照を generic に

現状: "Mirrors `mkgp2-patch/tools/hsd/hsd_export_for_blender.csx` exactly so the …"

修正案: 「HSDLib `HSDRawFile` JSON-equivalent dump (read-side parity gate)」程度に書き換え。csx ↔ Rust の対応を残したいなら別 doc (例えば `docs/notes/csx_export_parity.md`) に切り出して module doc は library 機能の説明に専念。

### 🔴 3. preset / setter docstring の "corpus 一般化主張" を弱める

対象:
- `MObj::allocate_textured` docstring の「widely seen on textured POBJs across the HSD vanilla course corpus」
- `Pobj.flags` getter docstring の "real-world game corpora sometimes repurpose these bits — most commonly 0x8000 on statically-bound textured POBJs"
- `set_lightmap_diffuse` 等 docstring の corpus 言及

修正案: 「one observed runtime convention」「a common course-mesh field pattern」「one such usage pattern」程度に弱める。1 ゲーム観察を library docstring で一般法則と書かない。corpus 統計は project memory (= mkgp2-patch session の memory) に置く。

### 🔴 4. preset の generic kwargs 化

現状: `MObj::allocate_unlit_color(r,g,b,a)` / `MObj::allocate_textured(material, image)` が固定 field 値を canned set。caller は値を上書きできない。

修正案: **generic version を kwargs で expose** — `MObj::allocate_textured(material, image, *, render_flags=…, tex_map_id=…, tex_gen_src=…, scale=…, wrap_s=…, wrap_t=…, repeat_s=…, repeat_t=…, mag_filter=…, color_op=…, alpha_op=…, blending=…, lightmap_diffuse=…)`。core 側は `TexturedPreset` / `UnlitColorPreset` struct を経由する `_with(preset)` 経路を追加し、既存 `_textured(material, image)` は `_with(.., Default::default())` 経由に。default 値は「lit single-texture surface」TEV stage layout — HSD の最も素朴な textured-render preset として純機能的に説明する (= 特定ジャンルの命名は library に入れない)。caller (mkgp2-patch addon 等) が course-genre 専用 wrap が欲しい場合は consumer 側で wrap fn を持つ。

PyO3 binding 同期 (kwargs を `#[pyo3(signature = (..., *, …))]` で expose)。

### 🔴 5. `Dat::alloc_scene_data` の命名/分割

現状: 「最小限の SObj → JObjDescs[1] → JObjDesc → root JObj」で refs=1。COBJ / LObj 不在 (memory entry `project_alloc_scene_data_lobj_cobj_trap.md`)。root 名前も `"scene_data"` hard-code。

修正案 (本タスクで実装した内容):
- `Dat::alloc_scene_data_minimal(root_name: &str)` に rename + root 名前を引数化 (HSDLib `HSDRootNode.Name` は free-form)。
- 既存 `Dat::alloc_scene_data()` は `#[deprecated]` alias として残し、`alloc_scene_data_minimal("scene_data")` を呼ぶ。
- PyO3 binding: `Dat.alloc_scene_data_minimal(root_name="scene_data")` を追加、`Dat.alloc_scene_data()` は `DeprecationWarning` を出す back-compat shim に。
- docstring で「minimal = no Cameras / Lights / Fog」と明示。

COBJ / LObj / Fog 込みの factory (`alloc_scene_data_with_camera_light` 等) は **本タスクのスコープ外**。SObj 側に COBJDesc / LObjDesc / FogDesc accessor の実装がまだ無く、その実装 + Tests 込みで別タスク扱い (= 後述 §3.5)。

### 🔴 6. `tests/parity.rs` の corpus generic 化 + env var rename

現状 (本タスク前): `MKGP2_FILES_DIR` / `MKGP2_PATCH_DIR` 名前 + 6 MKGP2 vanilla course + 3 alias-rich set hard-code。

修正案 (本タスクで実装した内容):
- env var:
  - 新 `HSDRAW_PARITY_CORPUS_DIR` (corpus dir; `.dat` ファイル群)
  - 新 `HSDRAW_PARITY_CSX_FILE` (csx ファイル直接 path)
  - 新 `HSDRAW_PARITY_FILES` (optional; comma-separated subset で gate 範囲を絞る)
  - 旧 `MKGP2_FILES_DIR` / `MKGP2_PATCH_DIR` は back-compat alias として残し、`MKGP2_PATCH_DIR` は `tools/hsd/hsd_export_for_blender.csx` の前提も含めて読む
- `target_files` hard-code を削除 → corpus dir 内 `*.dat` 全列挙 (sorted) を default に
- `survey_alias` example も同じ env var に追従
- README / mutation.rs / CI workflow のコメントを新 env var 名に書き換え
- 結果: library code に MKGP2-specific ファイル名 (`MR_highway_short_A.dat` 等) が残らない。consumer (mkgp2-patch CI) は `HSDRAW_PARITY_CORPUS_DIR=...; HSDRAW_PARITY_FILES=MR_highway_short_A.dat,...` を渡せば従来の gate を再現できる。

### 🔴 7. POBJ writer の attribute format を builder pattern に

現状 (本タスク前): 固定 POS F32×3 / NRM F32×3 / CLR0 RGBA8 / TEX0 F32×2。

修正案 (本タスクで実装した内容 = API 開放のみ):
- `pobj_writer::PosFormat` / `NormalFormat` / `ColorFormat` / `UvFormat` enum を追加。各 default は現状値 (F32x3 / RGBA8 / F32x2)。
- `MeshBuilder::set_pos_format` / `set_normal_format` / `set_color_format` / `set_uv_format` setter を追加。
- 内部 `encode_pos` / `encode_normal` / `encode_color` / `encode_uv` ヘルパで (バッファ生成 + AttrLayout {CompCnt, CompType, Stride}) を一手に算出。`AttrSpec` の三つ組はここから読む。
- 非 default variant (`S16x3` / `S8x3` / `Rgb565` / `Rgba4` / `S16x2` / `S8x2`) は **placeholder** として enum に席だけ用意し、`build()` は `"<attr> format <fmt> is not yet implemented in writer (todo.md §2.7)"` で error を返す。
- PyO3 binding に `MeshBuilder.set_pos_format(fmt[, *, exponent])` 等を expose。
- 既定値での setter 呼び出しが「setter なし」と byte-identical であることを round-trip test (`format_setters_with_defaults_match_no_setter_path`) で gate。

**本タスクスコープ外 (= 別 commit)**: 非 default variant の実装。具体的に必要なのは以下。
- `S16x3 { exponent }` / `S8x3 { exponent }` (POS / NRM) — `value × 2^exponent` で正負域フィット、`HSDLib GXCompType::S16 / S8` 値、`stride = 6 / 3`
- `S16x2 { exponent }` / `S8x2 { exponent }` (TEX0) — 同上、`stride = 4 / 2`
- `Rgb565` / `Rgba4` (CLR0) — RGB888 → R5G6B5 量子化、HSDLib `GXCompTypeClr::RGB565=0 / RGBA4=3`、`stride = 2`
- 各 format の round-trip test (`Dat::parse` → `gx_dl::unpack` → 同等性 within quantization tolerance)
- `gx_dl.rs` reader 側がこれらの format を decode できるかの確認 (GXCompType は枠組みとして decode 済 のはず — 念のため round-trip test で gate)

優先度: 🔴 (= 非 default 実装は fighter / character consumer 来訪時に必要)。

### 🔴 8. paletted format encoder

現状: CI4 / CI8 / CI14X2 / I4 / I8 / IA4 / IA8 が read-only。`encode_image` は呼び出されると todo.md §2.8 を引照するメッセージで error を返す (= 本タスクで明確化済)。

実装方針 (= 別 commit 扱い、未着手):

**(a) signature 設計**

palette 量子化が必要な CI4 / CI8 / CI14X2 と、palette 不要な intensity 系 (I4 / I8 / IA4 / IA8) で出力形が違う:
- intensity 系: `(format, w, h, rgba) -> image_bytes` で既存 `encode_image` に追加するだけ。RGB → luminance 変換だけで OK。
- paletted 系: `image_bytes` + `tlut_bytes` の 2 出力。`encode_image` の戻り値を `EncodedImage { image: Vec<u8>, tlut: Option<Vec<u8>> }` に拡張する案と、`encode_paletted_image(format, w, h, rgba) -> (image, tlut)` を別 fn として分離する案がある。後者の方が既存 caller 互換を保てる。

**(b) palette 量子化 algo 候補**
- `imagequant` (libimagequant Rust binding) — wu-quant + remap、pngquant 由来、高品質。ライセンス GPL3 / commercial。
- `color_quant` (median-cut) — Apache-2.0、軽量、品質はそこそこ。
- 自前実装 (k-means + Floyd-Steinberg dither) — 依存なし、品質と速度のバランス。

→ ライセンス + 依存サイズの観点で `color_quant` 推奨。

**(c) TLUT 配線**

HSDLib `HSD_Tlut` は `(palette_bytes, format, name, num_colors)` を持つ。typed accessor は既存:
- `TObj::set_tlut_data(Some(tlut))` で TObj に bind
- `Tlut::set_palette_bytes` 等 (= 既存 / 追加要確認) で payload を埋める

**(d) round-trip テスト**

各 format ごとに `rgba → encode_paletted_image → decode_image(format, ..., tlut) → rgba'` で完全 round-trip (8x8 / 16x16 / 非整数 block サイズ) + `gx_decode_round_trip` パターンに揃える。

**(e) caller impact**

paletted encode ができても、現在の `MeshBuilder` / `Image` / `TObj` 経路は `tobj.set_image_data(image)` だけで `tlut` 配線は呼び出し側責務。`MObj::allocate_textured_with` の docstring 既に「TLUT は別途 attach 必要」と明示済 (= #3 で softening した内容)。

優先度: 🔴 (= "MKGP2 で paletted hit ない" を library 制限の根拠にしてはいけないという design philosophy 上の理由)。実装規模が大きい (3-5 PR 分) ので、actual consumer 要望が来てから rollout する余地もあり、本セッションでは API 表面の整理 + 実装ガイダンスのみ。

### 🔴 9 [リスト的には番号 9, 当初分類は低]. POBJ writer の writer.rs 機能制限の generic 化

writer.rs の以下の機能が "course .dat" 前提で skip されている (= 他 HSD-format consumer に対して silently 動かない):

- `_nextStruct` ordering hack for shape anims
- SBM_FighterData / MEX_Data / kexData dedup suppression
- `Roots[0]` typed as MEX/kex disabling `bufferAlign`
- subaction orphan goto-pointer 修復

修正案: これらを opt-in flag 付きで実装する。e.g.
```rust
WriterOptions {
    fighter_data_dedup_suppression: bool,  // SBM/MEX/kex 互換
    shape_anim_next_struct_ordering: bool,
    ...
}
```

優先度: ⚪ (= **唯一の low priority**。fighter / MEX consumer の要望が来てから対応で OK)。

---

## 3. その他 — open items

### 3.1 mkgp2-patch から持ち越した `tasks/todo.md` 由来

| ID | 内容 | 優先度 |
|---|---|---|
| **A-2** | API consistency: `JObj.dobj()` getter 不在 / `DObj.mobj` raw HsdStruct vs `DObj.next` wrapper の戻り値不整合 | 🟡 |
| **A-3** | Linux/macOS wheel build (`.github/workflows/wheels.yml` を trigger するだけ; 要望ベース) | 🟡 |

### 3.2 既存の docs/roadmap.md からの引き継ぎ

`docs/roadmap.md` の冒頭文も MKGP2-leverage 順という framing なので、上記 #1〜#8 完了後に general ordering に書き直す。中身の各 phase 記述も "course mesh" 言及を見直し。

### 3.3 HSD_TOBJ_TEV / HSD_AObj / HSD_LObj / HSD_FObj / HSD_TexAnim 系 typed accessor

現状 `tev_data()` 等は `Option<StructRef>` 返し。typed accessor を build したい消費者が出たタイミングで増設。読み手の利便性は中ぐらい。

### 3.5 SObj に COBJDesc / LObjDesc / FogDesc accessor 拡張

`crates/hsdraw-core/src/common.rs` の `SObj` accessor は現状 `jobj_descs`(0x00) しか expose していない。HSDLib `HSD_SOBJ.cs` には 0x04 / 0x08 / 0x0C の Camera / Lights / Fog 配列フィールドがあり、これらに対応する `COBJDesc` / `LObjDesc` / `FogDesc` typed accessor + getter / setter が無いと "Camera / Light 付き SObj" を library 側で組めない。

実装後に `Dat::alloc_scene_data_with_camera_light(...)` factory を追加する。優先度 🟡 (= consumer から要望が来てから着手で OK)。

### 3.4 `MeshBuilder.from_arrays` の numpy zero-copy 経路

現状 `Vec<f32>` extraction (Python list / sequence / array.array / bytes 受け入れ)。本格的な numpy zero-copy が欲しければ `rust-numpy` 依存追加で `PyArray1<f32>` 直接受けに拡張可能。本 todo の `de-coupling` とは独立した perf 改善案。

---

## 4. 実装順序 (提案)

1. **🔴 1 / 2 / 3 を 1 commit でまとめる** — README / module doc / docstring 文言の差し替えだけで diff が浅い、merge 衝突も起きにくい。
2. **🔴 4 / 5** — preset rename + 既存 alias 経由の deprecation。caller (mkgp2-patch addon) のリファクタが必要なので別 commit。
3. **🔴 6** — test 経路の rename と分離。CI workflow も合わせて修正。
4. **🔴 7** — POBJ writer attr format builder。実装規模大 (writer の AttrSpec / DL emit / per-vertex byte layout を全部 format-aware にする)。round-trip test を format ごとに増やす必要あり。
5. **🔴 8** — paletted encoder。palette 量子化 algo 選定 + 依存追加 + TLUT 配線テスト。
6. **⚪ 9** — fighter / MEX / kex 互換 flag 実装。要望待ち。

ベンチマーク的には 1〜3 が即直し、4〜6 が API breaking refactor、7〜8 が新機能拡張、9 は requested-only。

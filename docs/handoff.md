````markdown
# HSD .dat reader/writer — Rust + PyO3 実装タスク

## 背景

MKGP2 (Mario Kart Arcade GP2) のコース mesh / texture は HSD (HAL System
Development) format の .dat に格納されている。SSBM と共通 format で、
parser として実用品は C# の HSDLib のみ存在する。

現状は csx (dotnet-script + HSDLib) 経由で .dat → JSON+PNG bundle に変換し
Blender addon (`mkgp2-patch/tools/blender/blender_addon_mkgp2_course/`) に
食わせている (read-only L1 pipeline)。これを **dotnet 依存なし**で動かし、
将来 Blender からの **write-back** (mesh 編集を .dat へ反映) に進めるために
Rust で実装し、Blender 同梱 CPython から import できるよう PyO3 binding を
生やす。

## ゴール

1. **Reader**: MKGP2 が使う HSD subset を Rust で parse、JSON+PNG bundle に
   変換できる (現 csx の出力と互換)
2. **Writer**: JSON+PNG bundle から .dat を書き戻せる (round-trip 同義性)
3. **PyO3 binding**: Blender 4.3 (CPython 3.11) から `import hsdraw` で
   使える、`abi3-py37` で前方互換
4. **Blender addon 配布**: pre-built `.pyd` / `.so` を addon zip に
   vendor できる (Win/Mac/Linux × x86_64/arm64)

## スコープ (MKGP2 で実需要があるものだけ)

含む:
- DAT header + relocation table + struct identity 管理
- scene_data + 公開 JObj root 解決 (alias root 含む)
- JObj / DObj / MObj / TObj / PObj
- GX texture decode/encode: **CMP / RGB5A3 / I8 / IA4 / RGBA8** (MKGP2 実用)
- DL (Display List) unpack: position / normal / texcoord / color /
  matrix index
- Joint forward kinematics + SingleBoundJObj

含まない:
- AObj (animation) — MKGP2 のコースは static
- IObj (image obj for chains) — 不要
- 汎用 SSBM 全機能 — depth 分対応で数ヶ月仕事になる

## レポジトリ

- 場所: `~/src/github.com/naari3/hsdraw/` (新規)
- 仮名 `hsdraw`、自由に変えて良い (`hsd-rs` / `pyhsd` 等)
- workspace 構成 (推奨):
  - `crates/hsdraw-core/` — pure Rust、std minimal 依存
  - `crates/hsdraw-py/` — PyO3 binding (`abi3-py37`、maturin ビルド)
  - `crates/hsdraw-cli/` — `hsdraw-cli decode foo.dat out/` 動作確認 / 
    round-trip テスト用 single-binary
- ライセンス: mkgp2-patch (MIT) と整合 (好みで MIT / Apache-2.0 dual)
- 完成後に Blender addon zip へ binary を vendor する形で結合

## 参考材料

絶対に読むこと:
1. **HSDLib のソース** (Ploaj/HSDLib)。`ghq get Ploaj/HSDLib` で clone。
   format 正規 reference は HSDRaw 配下の C#。特に:
   - `HSDRaw/HSDStruct.cs` — struct + reference table
   - `HSDRaw/HSDRawFile.cs` — file 全体の Save/Load (writer dedup ロジックは
     ここの `Save` メソッド内 `_structCache` + `structToOffset` 部分)
   - `HSDRaw/Common/HSD_JOBJ.cs` 等 — accessor 群
2. `mkgp2-patch/mkgp2docs/hsd_to_blender_visual_pipeline.md` — 既に踏んだ
   罠 5 件と JSON bundle shape
3. `mkgp2-patch/tools/hsd/hsd_export_for_blender.csx` — 現 csx 実装。
   出力 JSON 構造の唯一の正しい定義。Rust 側は **同じ形** を吐くこと
4. `mkgp2-patch/tools/hsd/hsd_add_alias_root.csx` — alias root 追加の
   最小 demo (HSDLib API レベルの挙動確認用)

参考にすると速い:
- `mkgp2-patch/mkgp2docs/mkgp2_course_layout_system.md` — MKGP2 コース構造

## テストデータ

vanilla MKGP2 .dat を round-trip テスト corpus に使う。生 ROM 由来なので
**repo に含めない**、絶対パスで参照する形にすること:

- `C:\Users\naari\Documents\Dolphin ROMs\Triforce\mkgp2\files\`
  - `mr_highway_short_A.dat` (短尺、≈700KB)
  - `mr_highway_long_A.dat` (長尺、≈2.5MB)
  - 他 30+ コース

`tests/data/` に小さい合成 .dat (数 KB) を 1〜2 個自作して repo に commit
すれば CI で round-trip 回せる。

## csx parity tests (必須要件)

新実装の正しさは csx (`mkgp2-patch/tools/hsd/hsd_export_for_blender.csx`)
の出力と **JSON 構造一致 + PNG 画素一致** であることで担保する。HSD format
spec が断片的でゼロから正しさを証明するのは現実的でないため、既に
Blender 経路で動いている csx を golden として扱う。**parity test を欠いた
状態で merge してはいけない**。

### test harness の最低要件

`tests/parity/` 配下に以下を置く (`cargo test --test parity` で回せる):

1. **driver**: .dat を入力に取り csx と Rust 双方を走らせて出力 dir を
   diff する。両者の `scene.json` と `tex/*.png` を比較
2. **csx 起動**:
   `dotnet-script $MKGP2_PATCH_DIR/tools/hsd/hsd_export_for_blender.csx
   <dat> <out>` を `std::process::Command` で呼ぶ。`dotnet-script` が
   無い / `MKGP2_PATCH_DIR` 未設定なら `eprintln!("skipped"); return;` で
   SKIP (test 自体は PASS 扱い)。CI では dotnet SDK を install して常時
3. **比較ルール**:
   - `scene.json`: `serde_json::Value` で両側読んで semantic diff。
     key 順は不問、float は `f64` 値で eps `1e-5` 比較。配列は per-key で
     順序依存判定。差分があれば「最初に divergence する path」を `panic!` で出す
   - `tex/*.png`: 全 byte 一致を要求 (両側 RGBA で出る設計)。差があれば
     `tests/parity/artifacts/<test>/` に left.png / right.png / diff.png を
     吐いて CI artifact に上げる
4. **対象 corpus**:
   - `tests/data/synthetic_*.dat` — repo commit。**必須 / CI 常時実行**
   - vanilla MKGP2 .dat — `MKGP2_FILES_DIR` 指定時のみ `#[ignore]` を
     外す形に。`cargo test --test parity --ignored` で個人環境確認、
     `MR_highway_short_A.dat` / `_long_A.dat` 等 6 ファイル最低カバー

### parity test を書くときに踏みやすい罠

- **HSDLib channel order**: csx 側は HSDLib の `GetDecodedImageData()` が
  CMP/RGBA8 で BGRA を返す癖を post-swap で吸収済。Rust 側も常に RGBA で
  出すので結果は一致する。PNG byte mismatch を見たら最初に Rust 側
  decoder の channel 順を疑う
- **JSON 浮動小数の文字列化**: C# `double.ToString()` と Rust `f64::to_string()`
  は同値でも文字列が違う。必ず numeric 比較、string 比較しない
- **alias root の dict 順**: HSDLib の `Dictionary<string,...>` は insertion
  順、Rust の `HashMap` はランダム。`scene["objects"]` は key set + 値の
  個別比較で順序非依存に
- **PNG metadata**: `image` crate のデフォは PNG に gAMA/pHYs を書き込む
  ことがある。csx 出力と byte 一致させるには encoder を最小構成で起こす

### CI 構成

GitHub Actions で Linux (ubuntu-latest) + macOS (macos-latest) +
Windows (windows-latest) のマトリクス。各 job で `dotnet-script` install →
`MKGP2_PATCH_DIR` を repo の submodule や fetch で確保 →
`cargo test --test parity` を必須。vanilla MKGP2 corpus は CI 配布できない
ので `--ignored` test は個人環境専用、PR-gate には含めない (synthetic で
カバー前提)。

## struct identity と alias root の設計拘束

HSDLib の alias root 機能 (HSDRawViewer の右クリック「Add Reference To Root」)
は次のように実装されている。Rust 移植も同形を保つこと:

**HSDLib 側の仕組み** (`HSDRaw/HSDRawFile.cs:584` Save):
1. `_structCache` は `List<HSDStruct>` で、`Contains` は identity (参照) 比較
2. `Roots` の各要素は `HSDRootNode { Name, Data: HSDAccessor }`、
   Data._s は HSDStruct インスタンス
3. 2 つの HSDRootNode が **同 HSDStruct インスタンス** を持っていれば
   `_structCache` には 1 entry しか入らず、書き出しも 1 回
4. Root 表書き出しで `structToOffset[root.Data._s]` を引いて offset を
   書く → 両 root が同 offset を指す = alias 完成

別物の最適化として `RemoveDuplicateBuffers()` がある。これは buffer 限定
(`References.Count == 0 && length > 0x40`、texture 等) で **バイト列 hash
ベースの dedupe**。alias root の identity-based 仕組みとは独立。

**Rust 設計拘束**:
- 内部 struct は `Rc<RefCell<HsdStruct>>` または `Arc<RwLock<HsdStruct>>`
  で持つ (`Box` 不可、identity が保てない)
- writer の offset map は **pointer identity key** で引く
  (`IndexMap<*const HsdStruct, u32>` または `Rc::as_ptr` ベース)
- `Roots: Vec<(String, Rc<HsdStruct>)>` に同 Rc を 2 entry 入れるだけで
  alias は自動成立する設計にする。alias 専用の特殊コードを書かない
- `remove_duplicate_buffers()` 相当はバイト列 hash で別実装、optional
  最適化として alias root 機能から分離する

parity test では alias root を持つ vanilla .dat (HSDLib sample や
SSBM jobj alias 構造) で round-trip し、両 root が同 offset を指すことを
byte レベルで確認する。

## 既知の罠 (HSDLib の癖、移植時に踏まないように)

1. **GX texture channel order**: `GetDecodedImageData()` は CMP / RGBA8
   だけ BGRA を返す、他 format は RGBA。Rust 実装は **format に
   よらず常に RGBA で出す** こと
2. **Alias root**: 上記セクション参照。identity-based 管理が必須
3. **TObj.Blending**: テクスチャの blend mode (Replace / Modulate / Decal /
   Blend 系)。読まないと BLEND が白被り。Rust も必須
4. **DL unpack**: GX vertex format は per-attribute で
   direct/index8/index16 が混在、attribute mask は GX_VAT で定義。
   HSDLib の DL 実装を読み下して Rust に再表現するのが速い

## 出力 JSON shape

`tools/hsd/hsd_export_for_blender.csx` の出力と **byte-identical 互換**で
ある必要はない (key 順は不問) が、Blender addon
(`tools/blender/blender_import_hsd.py`) が読める shape を保つこと。
`scene["objects"]`, `obj["mesh"]["vertices"]`, `obj["material"]["textures"]`
等を直接参照しているので、現 csx が吐く JSON を 1 つ pretty-print して
受け取り側の addon コードと突き合わせて schema を確定する。

## API スケッチ (参考、自由に変えて良い)

```rust
// hsdraw-core
pub struct Dat { /* parsed tree */ }
pub fn parse(bytes: &[u8]) -> Result<Dat, Error>;
impl Dat {
    pub fn write(&self) -> Vec<u8>;
    pub fn scene_root(&self) -> Option<&JObj>;
    pub fn public_roots(&self) -> &[(String, Rc<HsdStruct>)];
}
pub struct JObj { /* hierarchy node */ }
pub struct DObj { /* draw object — material + mesh */ }
pub fn decode_texture(tobj: &TObj) -> Image; // 常に RGBA8
pub fn encode_texture(img: &Image, fmt: GxFormat) -> Vec<u8>;
```

```rust
// hsdraw-py (PyO3)
#[pymodule]
fn hsdraw(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse_dat, m)?)?;
    m.add_function(wrap_pyfunction!(write_dat, m)?)?;
    Ok(())
}
```

```python
# Blender addon 側で最終的にこう使えるのが目標
import hsdraw
dat = hsdraw.parse_dat(open("foo.dat", "rb").read())
for name, root in dat.public_roots():
    for jobj in root.iter_descendants():
        for dobj in jobj.dobjs():
            mesh = dobj.unpack_mesh()
            tex = dobj.material().texture(0).decode_rgba8()
```

## 進め方の推奨

1. **Phase 0**: HSDLib clone + 小 .dat の hex 観察 + csx 出力 JSON で
   format 感覚を掴む (1 日)
2. **Phase 1**: Reader core (DAT header + relocation table + JObj tree)、
   `hsdraw-cli decode foo.dat` で stdout dump できる (3〜4 日)
3. **Phase 2**: GX texture decode、PNG 出力できる (2〜3 日)
4. **Phase 3**: DL unpack、各 mesh を OBJ/glTF で吐ける (3〜4 日)
5. **Phase 4**: parity test harness 整備 + Rust の JSON+PNG 出力を csx と
   一致させる。`tests/data/synthetic_*.dat` で `cargo test --test parity`
   全 PASS、続いて MKGP2 vanilla 6 コースで `--ignored` PASS。**ここを
   通過した時点で Phase 1〜3 の reader 正しさが担保される**。PyO3 binding は
   最後に被せる (3〜4 日)
6. **Phase 5**: Writer (relocation table 再構築、struct identity dedup +
   buffer hash dedup、書き戻し round-trip 通す)。alias root を持つ .dat の
   round-trip parity が rubber-meets-the-road (5〜7 日)
7. **Phase 6**: maturin + cibuildwheel で 6 platform wheel を CI build、
   Blender addon zip に vendor する手順整備 (2 日)

合計 〜3 週間が目安。Phase 5 が最難所、Phase 1/2/3 は素直、Phase 4 が
正しさの gate。

## 完了条件

- [ ] `hsdraw-cli decode mr_highway_short_A.dat out/` が動き、out/ に
      `scene.json` + `tex/*.png` が出る
- [ ] `cargo test --test parity` が `tests/data/synthetic_*.dat` で 100%
      PASS、Linux/macOS/Windows の CI 全 OS で green
- [ ] `MKGP2_FILES_DIR=...; cargo test --test parity --ignored` で MKGP2
      vanilla の MR_highway 短/長, mc_jungle, mc_kingdom, mc_palace,
      st_pyramid (6 コース) 全てで PASS
- [ ] PNG mismatch 時は artifact が自動で出る (CI 上のデバッグが回る)
- [ ] mkgp2-patch の Blender addon が `hsdraw` import 経由で vanilla
      course 6 種を import 成功。csx 経由と視覚的に一致するだけでなく、
      parity test で JSON/PNG byte レベルでも一致を確認済み
- [ ] writer round-trip: `decode → encode` で再生成した .dat を
      もう一度 decode した結果が初回 decode と semantic equivalent。
      alias root を持つ .dat も round-trip で alias 関係が保たれる
- [ ] PyPI 公開 / 内部配布、または Blender addon zip に platform binary
      vendor 手順確立

## 質問や仕様確認

mkgp2-patch repo (`naari3/mkgp2-patch`) の Issue で。HSDLib 既知の癖や
MKGP2 特有の format quirk は把握しているので、読み下しに詰まったら遠慮なく
聞くこと。
````
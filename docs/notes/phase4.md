# Phase 4: parity test harness + csx 一致

Phase 1〜3 の reader が csx golden と意味的に一致することを保証する gate。
Phase 4 を通過した時点で reader 層の正しさが担保される。Phase 5 の
writer はこの reader を逆向きに駆動する形で組む。

## 構成

- `crates/hsdraw-core/tests/parity.rs` — harness 本体
  - `json_diff_recognizes_eps`: ε=1e-5 の float 比較が機能することの自己チェック
  - `rust_export_runs_on_synthetic`: in-memory で組んだ最小 .dat (≈0x48B、root 1) で
    `Dat::parse` + `export::export_scene` が走ることを確認する CI gate
  - `vanilla_corpus_round_trips`: env 変数 `MKGP2_PATCH_DIR` / `MKGP2_FILES_DIR` が
    両方セットされていて `dotnet-script` が PATH 上にあるときだけ起動する。csx と
    Rust の両方を同じ .dat で走らせ、`scene.json` を意味比較、`tex/*.png` を
    pixel-equal 比較する。
- `MKGP2_PATCH_DIR=...; MKGP2_FILES_DIR=...; cargo test --test parity vanilla_corpus_round_trips`
  で 6 ファイル全 PASS:
  - `test_course_start_gate.dat`
  - `MR_highway_short_A.dat` / `MR_highway_long_A.dat`
  - `DK_jungle_short_a.dat` / `DK_jungle_long_a.dat`
  - `AT_demo.dat`

handoff の "MR_highway 短/長, mc_jungle, mc_kingdom, mc_palace, st_pyramid" は
仮想的なファイル名のため、実在する MKGP2 vanilla の代表 6 ファイルを採用した。
CMP / RGBA8 / RGB5A3 / IA8 / I8 を含む幅広いテクスチャ format と、複雑な
JObjDesc tree (DK_jungle_long_a) を網羅する。

## 比較ルール

`docs/notes/phase0.md` §4 "diff rules" に従う:

- **JSON**: key 順序は無視 (BTreeMap で整列)、配列は positional、float は
  ε=1e-5 (相対 / 絶対のいずれかが満たされれば PASS)、整数は exact。
- **PNG**: byte equality を最優先。byte が異なる場合は両方デコードして
  RGBA8 で pixel equality を確認 (deflate 実装差で byte は揺れるが
  pixel は揺れない)。

## CI artifact

PNG 比較で違いが出ると `target/parity/<dat>/artifacts/{csx,rust}/` に元
PNG がコピーされる。失敗時 panic メッセージにこの path が出るので、CI 側で
`actions/upload-artifact` でこのディレクトリを引き上げれば、ローカル再現
なしで triage できる。`.gitignore` で `target/` 配下なので追跡されない。

## RGB5A3 のチャネル順 (重要な落とし穴)

HSDLib `GXImageConverter.fromRGB5A3` のローカル変数命名はトラップ:

```cs
// RGB555 case
b = (pixel >> 10) & 0x1F   // top 5 bits
g = (pixel >> 5)  & 0x1F   // mid 5 bits
r = pixel        & 0x1F    // bottom 5 bits
```

つまり HSDLib の `r` は pixel の **下位** ビット、`b` は **上位** ビット。
出力は `(r << 0) | (g << 8) | (b << 16) | (a << 24)` で u32 にパックされ、
PNG の RGBA8 byte streams としては (r,g,b,a) の順に LE で展開される。
csx は RGBA8/CMP では post-swap で R↔B するが、**RGB5A3 と RGB565 では
post-swap しない**。よって PNG byte 0 は HSDLib の `r` (= pixel 下位 5/4 bit)。

`crates/hsdraw-core/src/gx_image.rs::decode_rgb5a3` はこの label を踏襲して
書いている。素直に "上位 5bit を r" と書くと csx と R/B が入れ替わる。
RGB565 path も同じ命名規則 (`r = pixel & 0x1F`) なので確認できる。

## PyO3 binding (最低限の表面)

`crates/hsdraw-py/src/lib.rs`:

- `hsdraw.version() -> str`
- `hsdraw.parse_dat(bytes) -> Dat` (root 名一覧と byte_size を持つハンドル)
- `hsdraw.export_scene_json(bytes, source_dat="", tex_dir=None) -> str`
  → csx と等価な scene JSON を文字列で返す。`tex_dir` 指定時は PNG も書き出す。

`crates/hsdraw-py/pyproject.toml` で maturin develop / build できる。
abi3-py37 wheel を 1 個作れば 3.7+ どの CPython でも動く。
詳細な object-graph API (`dat.public_roots()` 等) は Phase 5 (writer) 後に
固める — alias 関係まで決まってから surface を凍結する。

## Phase 4 で意図的に保留

- `tests/data/synthetic_minimal.dat` の実体ファイル化:
  Phase 5 の writer ができれば自然に生成できる (いまは harness が in-memory で
  組み立てている)。Writer が出来た直後にコミット。
- 6 ファイルを越える広域 corpus への適用:
  Phase 4 gate は "代表 6 種で PASS"。Phase 5 で writer round-trip を全 vanilla
  に拡大する際にまとめて。

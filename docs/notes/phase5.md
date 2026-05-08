# Phase 5: Writer + alias-root round-trip

reader が csx と意味的に一致することは Phase 4 で gate 済み。Phase 5 では
逆方向 (`Dat::write`) を実装し、`parse → write → parse` で alias topology
含めて round-trip 可能であることを確認する。

## 実装位置

- `crates/hsdraw-core/src/writer.rs` — `Dat::write` / `WriteOptions`
- `crates/hsdraw-cli/src/main.rs` — `encode` subcommand
- `crates/hsdraw-py/src/lib.rs` — `hsdraw.write_dat(bytes, ...)`
- `crates/hsdraw-core/tests/parity.rs` — `vanilla_corpus_writer_round_trips`
- `crates/hsdraw-core/tests/data/synthetic_minimal.dat` — committed fixture
  (regen: `cargo run -p hsdraw-cli --example gen_synthetic`)

## アルゴリズム (HSDLib `HSDRawFile.cs::Save` 準拠)

1. **gather**: roots + references から DFS で reachable な struct を
   identity-dedupe で全列挙
2. **cache 構築**: 既存 `dat.struct_order` を `optimize` フィルタで残し、
   未登録の reachable struct を末尾追加 (buffer の場合は先頭に insert)
3. **buffer dedup** (optimize 時): `IsBuffer` 判定 + `CanBeDuplicate` の
   struct を C# 互換 FNV-1a-mix ハッシュで集約。同一ハッシュの 2 個目以降は
   1 個目に redirect、参照を全 cache 走査で書き換え、cache から除去
4. **emit**:
   - 0x20 byte zero header (後で書き戻し)
   - 各 struct を順に書く: `IsBuffer && buffer_align` → 0x20 align、
     `align` (default true) → 4 align、それ以外は align なし
   - struct 識別子 → byte offset の `HashMap<*const RefCell, u32>`
   - 4-align で reloc table 開始
   - 各 struct.references を走査:
     - 該当 byte slot に target offset を上書き
     - reloc table に absolute pointer 位置を追加 (例外: ref-chain 非先頭の
       key=0 はリストに加えない — singly-linked alias-chain 仕様)
   - reloc table → root/ref symbol entries (pad で先確保) → string pool
   - header を最終書き戻し (filesize, reloc_offset_rel, reloc_count,
     root_count, ref_count, version[4])

## IsBuffer 判定 (= HSDLib と同一)

```rust
fn is_buffer(s: &HsdStruct) -> bool {
    if !s.can_be_buffer { return false; }
    (s.references().is_empty() && s.len() > 0x40) || s.is_buffer_aligned
}
```

reader 側で `len > 0x40 && refs 空` の struct には parse 時点で
`is_buffer_aligned = true` を立てている (texture 等の自動検出)。

## Alias 関係の保持

vanilla MKGP2 の `*_set.dat` (waluigi/yoshi/wario) や course .dat は、
top-level の `*_joint` root が SOBJ tree 内の JObj を Rc で **共有** する
パターンを多用する (`mkgp2docs/hsd_alias_and_blender_pipeline.md`)。

- reader: `parse` で同じ struct offset を 2 回見つけたら、`HashMap` 経由で
  同じ `Rc<RefCell<HsdStruct>>` を再利用。これで identity が一致
- writer: `structToOffset` (= `offset_of` HashMap) を identity でキーにする
  ので、複数 root が同じ Rc を持てば 1 個の byte offset に集約される

`vanilla_corpus_writer_round_trips` の `alias_topology()` は
"root j の data が、別の root i の sub-struct 内に存在するか" を順序固定の
リストとして抽出する。round-trip 前後でこのリストが完全一致することを
9 corpus ファイル (test_course_start_gate / MR_highway 短長 / DK_jungle 短長 /
AT_demo / waluigi_set / yoshi_set / wario_set) で確認済み。

```
✓ MR_highway_long_A.dat writer round-trip OK (16 alias roots)
✓ waluigi_set.dat writer round-trip OK (12 alias roots)
✓ wario_set.dat writer round-trip OK (12 alias roots)
```

## バイト一致を gate にしない理由

HSDLib `Save` 自身が:

- struct cache の挿入順を declaration ⇄ buffer-front で並び替える
- reloc table を `_structCache` × `references` の dictionary 順で書く
  (BTreeMap 順 = offset 昇順、HSDLib の `Dictionary<int,...>` の挿入順とは
  別物だが reader はどちらの順でも正しく parse する)
- 4-align / 0x20-align の判定が `IsBuffer` 推測に依存

これらの組み合わせで、HSDLib 自身でも 2 回 Save した結果が byte-equal にならない。
だから "scene.json semantic 同一 + alias topology 同一" を gate にしている。

## skipped (Phase 5 範囲外として保留)

- `_nextStruct` ordering hack (shape anim 用、MKGP2 では未使用)
- `SBM_FighterData` / `MEX_Data` / `kexData` の dedup 抑制 (course .dat 範囲外)
- "subaction orphan goto-pointer" 修復 (DEBUG ビルド限定の orphan 表示)
- accessor 単位の `Optimize` (`writer.trim` フラグの中身)

これらが必要になるのは Smash Melee / KAR-Ext の編集ユースケースを取り込むとき。

## CLI / Python の使い方

```bash
hsdraw-cli encode foo.dat --out foo_rewritten.dat
```

```python
import hsdraw
with open("foo.dat", "rb") as f:
    raw = f.read()
rewritten = hsdraw.write_dat(raw)               # 既定で optimize + buffer_align
faithful  = hsdraw.write_dat(raw, optimize=False, buffer_align=False)
```

Phase 6 で Blender addon が `hsdraw.write_dat` 経由で saved DAT を作る前提。

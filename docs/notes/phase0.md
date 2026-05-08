# Phase 0 — format reading notes

実装に入る前に HSDLib (Ploaj/HSDLib, `~/src/github.com/Ploaj/HSDLib`) と csx
(`~/src/github.com/naari3/mkgp2-patch/tools/hsd/hsd_export_for_blender.csx`)
を読み下した結果。これ以降の Phase はここで決めた解釈を前提にする。

参考 doc 群 (handoff の `mkgp2docs/...` は `~/src/github.com/dolphin-emu/dolphin/mkgp2docs/`
に置かれている):

- `dolphin-emu/dolphin/mkgp2docs/hsd_to_blender_visual_pipeline.md` — 罠 6 件 + JSON shape
- `dolphin-emu/dolphin/mkgp2docs/hsd_alias_and_blender_pipeline.md` — alias root
- `dolphin-emu/dolphin/mkgp2docs/hsd_parsing_tools.md` — HSDLib accessor 表
- `dolphin-emu/dolphin/mkgp2docs/mkgp2_course_layout_system.md` — コース構成
- `naari3/mkgp2-patch/tasks/lessons.md` 2026-05-07 — channel-order の罠

## 1. DAT header / relocation table 仕様

`HSDRaw/HSDRawFile.cs:Open` を ground truth に取る。すべて big-endian、struct
本体の絶対 offset はすべて **+0x20** 補正が要る (file header は 0x20 bytes、
struct data はその後ろから始まる)。

```
+0x00 u32  fsize           // 全体ファイルサイズ
+0x04 u32  relocOffset_rel // 0x20 を足して絶対 offset
+0x08 u32  relocCount
+0x0C u32  rootCount
+0x10 u32  refCount
+0x14 char[4] versionChars // 4 byte ASCII (rb_light_Z.dat は 0x00000000 で空も観測)
+0x18..0x20  ゼロ padding (8 bytes)
+0x20..relocOffset      struct data (連続配置)
+relocOffset            relocation table: u32[relocCount] 各要素は struct data 内
                         のフィールドの **相対 offset** (+0x20 で絶対)。指している
                         位置に格納されている値も相対 offset (+0x20 で絶対)。
+ ↑+relocCount*4        root table: rootCount × { u32 dataRel, u32 stringRel }
+ ↑+rootCount*8         ref  table: refCount  × { u32 dataRel, u32 stringRel }
+ ↑+refCount*8          string pool: NUL terminated UTF-8、stringRel はこの先頭
                         からの byte offset
```

確認用 hex (rb_light_Z.dat, 531 bytes; `xxd` 結果を `phase0_minimal_dat.md`
に貼る代わりにここに): `0000 0213` (fsize=0x213) → `0000 0100` (relocRel=0x100,
abs=0x120) → `0000 0015` (21 reloc) → `0000 0001` (1 root) → `0000 0000` (0 ref)
→ versionChars 全 0 → padding 全 0 → struct data → reloc table → root entry
→ string pool (`scene_data\0`)。

### Open() の罠

実装は HSDRawFile.cs:106 以降。Rust 移植時に踏みやすい点:

1. **objectOff > relocOffset の補正** (cs:148)。relocation がファイル末尾を指す
   "manually relocated" 補修済み .dat があるので、そのとき `Offsets` に
   `fsize` を append して struct 区間を切り出せるようにする。
2. **ref symbol の chain walk** (cs:192-216)。reference root の `Data` は
   その struct から `*data` を辿って 0/-1 終端まで chain がある (singly-linked
   ref array)。chain 中の各 struct を `relocOffsets` と `Offsets` に注入する
   特殊処理 — 通常 relocation table には現れない。
3. **Offsets 区切り**: `Offsets.Sort()` 後、隣接 offset の差分で struct を
   切り出す。最後の Offset は `relocOffset` または `fsize`。
4. **References ←→ struct mapping** (cs:283-314): `relocOffsets`
   (位置 → 指している先) を offset ごとに groupby、各 struct 内の reloc 位置
   `inner - structOffset` を key に `_references` に登録。
5. **orphan struct**: どの root/reference からも到達しない struct は
   原則無視されるが、HSDLib の DEBUG ビルドだと `Orphan0xXXXX` という
   仮 root を生やす (cs:371-374)。Release では無視。**Rust 側も orphan は
   無視** (round-trip では落とす方向で問題なし)。
6. **subaction goto hack** (cs:351-368): `0x1C000000` トークンで
   self-referential な末尾を持つ orphan があったら直前 struct に append
   する。これは AObj/SubAction 系で出るので MKGP2 コースでは恐らく不要、
   Phase 5 まで保留。

### parse 出力の最小構造

Rust 側で持つべき内部 model:

```rust
pub struct Dat {
    pub version: [u8; 4],
    pub roots:      Vec<(String, Rc<RefCell<HsdStruct>>)>,
    pub references: Vec<(String, Rc<RefCell<HsdStruct>>)>,
    // 解析時の入力順を保つため struct cache も Vec<Rc<RefCell<HsdStruct>>>
    pub struct_order: Vec<Rc<RefCell<HsdStruct>>>,
}
pub struct HsdStruct {
    pub data: Vec<u8>,
    /// key = struct 内 byte offset、value = 指している struct
    pub references: BTreeMap<u32, Rc<RefCell<HsdStruct>>>,
    /// IsBufferAligned (= 0x20-aligned in writer)
    pub is_buffer_aligned: bool,
    pub align: bool,           // default true (= 4-aligned)
}
```

`BTreeMap` は writer dedup と reloc table 出力時の順序安定化のため。
HSDLib の `Dictionary<int, HSDStruct>` は insertion 順だが Rust の
`HashMap` は非決定的なので、parity を確保したい場面では key sort を要求する。

## 2. 公開 root の解決 (Accessor 木の入口)

handoff の「scene_data + 公開 JObj root + alias root」の意味は次の通り:

- **scene_data root** は `HSD_SOBJ`、その `JOBJDescs[0].RootJoint` が JObj 木の
  ベース (`HSDRawFile.cs:818-821`, `HSD_SOBJ.cs:23` 経由)。MKGP2 コースは
  `JOBJDescs` 配列 1 個だけが入っているのが通常 (csx が `[0]` 決め打ちで動いて
  いる)。
- それ以外の root は `MR_highway_*_joint` 等の `_joint` 終わりで、`HSDRawFile`
  の `symbol_identificators` にて `HSD_JOBJ` accessor として読まれる
  (`HSDRawFile.cs:798`)。
- これら `*_joint` root の **多くは scene_data 配下の subtree への alias** で、
  同 `HSDStruct` インスタンスを共有している。csx は `HashSet<HSDStruct>`
  (= `jobjIdByStruct` dict) で重複 walk を回避し、追加 root が既に visited なら
  名前 → joint id だけを `joint_aliases` に記録する。

Rust 側の export 順 (csx と一致させる):

1. roots を順走、`scene_data` だけ最初に処理: `BuildWorld` + `EmitJoint` +
   `EmitMeshes` を一段で実行。
2. 残りの root を入力順で処理。`r.Data` が `HSD_JOBJ` 型のもの (= `*_joint`)
   のみが対象。`worldByJobj` は念のため `BuildWorld` を呼んで埋める
   (visited 判定込みなので二重計算しない)。
3. その JObj が `jobjIdByStruct` に既出なら `joint_aliases[r.Name] = exist_id`
   のみ記録 (joint emit と mesh emit を skip)。
4. 既出でなければ `EmitJoint` + `EmitMeshes` を回し、`joint_aliases[r.Name] = new_id`
   も記録 (= alias でなくても name → id の lookup table を提供)。

ここで `jobjIdByStruct` の key は **struct identity (= `Rc::as_ptr` ベース)**。
`HashSet<*const HsdStruct>` 相当。バイト列 hash でも丸ごと一致しないこと
があるので NG。

## 3. struct identity と alias root (writer 設計拘束)

HSDLib の `Save` (`HSDRawFile.cs:587-753`) のフロー:

1. `GetAllStructs()` で root/reference から到達する全 struct を **identity
   ベース重複なし** で集める (`Contains` は object reference 比較)。
2. 既存 `_structCache` のうち到達不能なものを drop。
3. 新規到達 struct を `_structCache` に追加 (buffer 判定のものは先頭、
   それ以外は末尾)。
4. (optimize=true で fighter/MEX 以外なら) `RemoveDuplicateBuffers()`:
   buffer 判定 (`References.Count == 0 && Length > 0x40`、または
   `IsBufferAligned`) で `IsRoot` でない struct を **byte hash で dedup**。
   alias root の identity 重複とは別経路。
5. shape anim 用 `_nextStruct` の隣接強制で並び替え。
6. `_structCache` を順に書き出し、`structToOffset[s] = pos` を埋める。
   buffer は `Align(0x20)`、それ以外 `s.Align ? Align(4) : nothing`。
7. relocation table 書き出し: 各 struct の references を走り、その struct の
   offset + 内部 key を relocation 位置として記録、書き先には参照先 offset を
   書く。
8. root/reference table 書き出し: `structToOffset[root.Data._s]` を引いて
   offset、string pool offset と組で出力。**alias root は同 `_s` を持つので
   同 offset を指すバイトが root entry に並び、自然に alias 関係が成立**。

### Rust 側の確定ルール

- `Vec<(String, Rc<RefCell<HsdStruct>>)>` の同 `Rc` を 2 entry 入れただけで
  alias 完成。
- offset map は `IndexMap<*const HsdStruct, u32>` (= `Rc::as_ptr() as *const _`
  を key)。`==` ではなく pointer 同一性で引く。
- `RemoveDuplicateBuffers` は **後段の最適化として独立実装**。読み込み済み
  バイト hash → 第二発見以降を first 出現に置換、`structCache` から第二以降を
  drop。alias root とは絶対に混ぜない (lessons.md 2026-05-07 の罠類縁)。

### parity test 上の確認方法

`hsd_add_alias_root.csx` と同等の Rust 版 CLI を Phase 5 で作って:

1. vanilla `MR_highway_short_A.dat` などを Rust で decode → scene.json + tex
2. Rust writer で .dat に書き戻し
3. 生成 .dat を **HSDLib 経由で再 open**、`GetOffsetFromStruct` で目的 alias
   2 root が同 offset を返すことを確認
4. 同じ .dat を Rust で再 decode、scene.json も alias 関係 (joint_aliases) が
   保たれていることを確認

これは Phase 5 の必須 acceptance test。

## 4. 出力 JSON の正準 schema

csx (`hsd_export_for_blender.csx`) が吐く DTO レコードを正準とする。
`scene` トップレベル (key 順 不問、配列要素は per-key で順序依存):

```jsonc
{
  "source_dat": "MR_highway_short_A.dat",
  "tex_dir":    "tex",
  "textures": [{
    "id":     "<sha1 12 hex>",          // 同 sha1 = 同 PNG file
    "file":   "tex/<id>.png",
    "width":  int, "height": int,
    "format": "CMP" | "RGB5A3" | "I8" | "IA4" | "RGBA8" | ...   // GXTexFmt 名
  }, ...],
  "materials": [{
    "id":               "mat_<n>",
    "render_flags":     "CONSTANT, TEX0, ALPHA_MAT",  // RENDER_MODE flags の
                                                       // ", " join。空 flag は除く
    "render_flags_raw": uint32,
    "diffuse_rgba":     [r,g,b,a],   // HSD_Material.DIF_*、各 0..255 int
    "alpha":            float,        // HSD_Material.Alpha
    "textures": [{                    // HSD_TOBJ chain (Next 順)
      "tex_id":     "<sha1>",
      "tex_map_id": "GX_TEXMAP0",
      "wrap_s":     "REPEAT" | "CLAMP" | "MIRROR",
      "wrap_t":     同上,
      "repeat_s":   int, "repeat_t": int,
      "mag_filter": "GX_NEAR" | "GX_LINEAR" | ...,
      "color_op":   "MODULATE" | "REPLACE" | "BLEND" | "RGB_MASK" | "ADD" | "NONE" | ...,
      "alpha_op":   "MODULATE" | "REPLACE" | "NONE" | ...,
      "blending":   float            // BLEND ColorOp の mix factor
    }, ...]
  }, ...],
  "joints": [{
    "id":           "jobj_<n>",
    "name":         null,             // 現状常に null (alias 名は joint_aliases 側)
    "flags":        ["OPA","ROOT_OPA",...], // JOBJ_FLAG 名、Split(", ") 後 trim
    "translation":  [tx,ty,tz],
    "rotation":     [rx,ry,rz],
    "scale":        [sx,sy,sz],
    "world_matrix": [16 floats],       // accumulated world、row-major (M11,M12,M13,M14,
                                       //  M21..M44)
    "parent":       "jobj_<n>" | null,
    "children":     ["jobj_<n>", ...]
  }, ...],
  "joint_aliases": {                   // root 名 → joint id (alias でない root 含む)
    "MR_highway_alpha_joint": "jobj_1",
    ...
  },
  "meshes": [{
    "id":                "mesh_<n>",
    "joint":             "jobj_<n>",   // この PObj をぶら下げる JObj
    "single_bind_joint": "jobj_<n>" | null,
    "material":          "mat_<n>" | null,
    "cull":              "FRONT" | "BACK" | "BOTH" | "NONE",
    "source_path":       "jobj_3/DObj0/PObj0",
    "vertices": [[x,y,z], ...],        // **world space baked** (= local *
                                       //  (parent.world * single_bind.world))
    "uvs":      [[u,v], ...],          // optional (TEX0 が 0 でない頂点があれば出力)
    "normals":  [[nx,ny,nz], ...],     // optional (NRM が 0 でない頂点があれば出力)
                                       //  rotation 部のみ transform、最後 normalize
    "colors":   [[r,g,b,a], ...],      // optional (CLR0 attribute があれば出力)、0..1 float
    "primitives": [{
      "type":    "Triangles" | "TriangleStrip" | "TriangleFan" | "Quads" | ...,
      "indices": [int, ...]            // mesh 内の vertex index、connected な
                                       // primitive 単位で 0,1,2,...,N-1 が cursor
    }, ...]
  }, ...]
}
```

### 受け取り側 (`blender_import_hsd.py`) が触る key

`blender_import_hsd.py` を読んで実際に参照されているのは:

- `scene["source_dat"]`, `["textures"][]`, `["materials"][]`, `["meshes"][]`,
  `["joint_aliases"]`, `["joints"]` (custom prop 用に collection に保存だけ)。
- `tex.{id, file}`, `mat.{id, render_flags, diffuse_rgba, alpha, textures[]}`,
  `texref.{tex_id, mag_filter, wrap_s, color_op, blending, alpha_op}` 
  (`wrap_t`/`repeat_s,t` は現状未参照だが将来用に出しておく)。
- `mesh.{id, joint, single_bind_joint, cull, source_path, vertices, uvs,
  normals, colors, primitives[].{type, indices}}`、`material`。

つまり `joints[]` と `joint_aliases{}` が無くても import 自体は成功するが、
custom prop 経由で round-trip 用に保持されるので必須。

### 浮動小数比較の方針

C# の `double.ToString()` と Rust の `f64::to_string()` は同値でも別文字列。
parity test では:

- `serde_json::Value` で両方 parse、`Number` を `f64::from` で取り出して
  `eps = 1e-5` 比較。NaN は両側 NaN なら equal 扱い。
- 配列は順序依存で per-element 比較、object は key set 比較 + 値の再帰比較。
- 最初に diverge した JSON Pointer (`/meshes/12/vertices/3/0`) を panic
  message に含める。

## 5. GX texture format 一覧と channel order の罠

| GXTexFmt | block | bytes/pixel | HSDLib decode 出力 byte order | csx で R↔B swap |
|---|---|---|---|---|
| I4   = 0  | 8×8 | 0.5 | (i, i, i, i)         | No |
| I8   = 1  | 8×4 | 1   | (i, i, i, i)         | No |
| IA4  = 2  | 8×4 | 1   | (i, i, i, a)         | No |
| IA8  = 3  | 4×4 | 2   | (i, i, i, a)         | No |
| RGB565 = 4| 4×4 | 2   | (HSDLib のラベルでは r=低5, g=中6, b=高5, a=255) | No (= csx は decoded を そのまま PNG 化) |
| RGB5A3 = 5| 4×4 | 2   | (r, g, b, a)         | No |
| RGBA8 = 6 | 4×4 | 4   | **(b, g, r, a) = BGRA**       | **Yes** |
| CI4  = 8  | 8×8 | 0.5 | palette 経由 (r,g,b,a) | No |
| CI8  = 9  | 8×4 | 1   | 同上                  | No |
| CI14X2=10 | 4×4 | 2   | 同上                  | No |
| CMP  = 14 | 8×8 (DXT1 ベース) | 0.5 | **(b, g, r, a) = BGRA**       | **Yes** |

**Rust 実装方針 (handoff §既知の罠 1 と整合)**: format によらず decoder は
**RGBA を直接出力**する。CMP/RGBA8 については HSDLib の packing をそのまま
真似ず、decode loop の中で `r ↔ b` を入れ替えた layout で書き出す。csx の
post-swap 相当を decoder 内に内包。

handoff スコープに載っているのは MKGP2 実需 5 つ: **CMP, RGB5A3, I8, IA4,
RGBA8**。Phase 2 ではこの 5 つを最低カバー、残り (I4, IA8, RGB565, CI*) は
Phase 4 の vanilla corpus で要請されたら追加 (現状の MR_highway_short_A.dat
には CMP/RGB5A3 が大半)。

### tile/swizzle 仕様

各 format は GX hardware の tile 単位で swizzle される (block size は上表)。
HSDLib decoder の loop nest を Rust に直訳 (テストは parity に丸投げ)。

## 6. DL unpack (PObj → vertex 配列)

`HSD_POBJ` (`HSDRaw/Common/HSD_POBJ.cs`):

```
+0x00 string ClassName (ref)
+0x04 HSD_POBJ Next
+0x08 HSD_GX_Attribute[] Attributes (GX_VA_NULL 終端)
+0x0C u16 Flags (POBJ_FLAG: CULLBACK/CULLFRONT/SHAPESET など)
+0x0E i16 DisplayListSize (× 32 で実 byte)
+0x10 byte[] DisplayListBuffer (buffer ref)
+0x14 union { HSD_ShapeSet | HSD_JOBJ SingleBoundJOBJ
            | HSDNullPointerArrayAccessor<HSD_Envelope> EnvelopeWeights }
    Flags & SHAPESET             → ShapeSet
    Flags & ENVELOPE             → EnvelopeWeights
    それ以外                     → SingleBoundJOBJ
```

`GX_Attribute` (`HSDRaw/GX/GX_Attribute.cs`、TrimmedSize 0x18):

```
+0x00 u32 AttributeName  // GXAttribName: GX_VA_POS=9, NRM=10, CLR0=11, TEX0=13, NULL=0xFF
+0x04 u32 AttributeType  // 0=NONE, 1=DIRECT, 2=INDEX8, 3=INDEX16
+0x08 u32 CompCount      // PosXYZ=1, NrmXYZ=0, ClrRGBA=1, TexST=1 等
+0x0C u32 CompType       // UInt8/Int8/UInt16/Int16/Float、CLR系は GXCompTypeClr で別解釈
+0x10 u8  Scale          // 値 = raw / (1 << Scale)
+0x12 i16 Stride
+0x14 ref HSDAccessor Buffer  // 連続 Stride × Count バイト
```

DL は per-PObj の bytecode:

```
loop:
  u8 PrimitiveType  (0=END / 0x80=Quads / 0x90=Triangles / 0x98=TriangleStrip
                    / 0xA0=TriangleFan / 0xA8=Lines / 0xB0=LineStrip / 0xB8=Points)
  u16 vertCount
  for vertCount:
    for each Attribute (NULL を除く):
      switch AttributeType:
        DIRECT: 通常は u8 だが、CLR0/CLR1 は CompType (RGB565=2, RGB8=3, RGBX8=4,
                RGBA4=2, RGBA6=3, RGBA8=4 byte) ぶん inline 読む
        INDEX8 : u8
        INDEX16: u16
```

`GX_VertexAccessor.GetDecodedVertices` で各 attribute index を Buffer から
decode、`GX_Vertex` 構造体 (POS, NRM, CLR0, CLR1, TEX0..TEX7, ...) に詰める。

### Phase 3 Rust 実装の要点

- attribute 配列の **末尾に必ず GX_VA_NULL** が来る (来なければ POBJ 不正)。
- `Attribute.Buffer` は別 struct への ref。Buffer.Length / Stride で count。
- `Scale` は **割る方** (raw / (1<<Scale))。signed 系は sign-extend してから。
- DIRECT で CLR0 を inline 読む際の compType 別 byte 数は GX 仕様に従う
  (`GX_PrimitiveGroup.ReadDirectGXColor` 参照)。
- 出力は csx と同じく per-primitive で `[0,1,2,...,N-1]` の cursor 連番
  indices を吐く (vertex 共有しないので mesh あたりの vertex 数 = primitive
  vertex 数の総和)。

### SingleBoundJObj と world transform

forward kinematics:

```
local(j)  = S(SX,SY,SZ) * R_xyz(RX,RY,RZ) * T(TX,TY,TZ)   // row-vector
world(j)  = local(j) * world(parent)
```

PObj の vertex を world に持っていく:

```
parentT = world(j_with_dobj)
sbT     = world(p.SingleBoundJOBJ) || Identity
finalT  = parentT * sbT
v_world = v_local * finalT
n_world = normalize( n_local * (finalT の M14/M24/M34 を 0 にしたもの) )
```

Euler は **XYZ 順、row-vector convention**。csx の `MatrixFromEuler` と完全に
一致する係数で組む必要がある (`docs/handoff.md` § 出力 JSON shape より、Rust
側 `f32` で OK だが計算順序を csx と揃える)。

## 7. 既知の罠 (lessons.md + visual_pipeline.md からの集約)

1. **Channel order**: §5 のとおり CMP/RGBA8 のみ HSDLib は BGRA を返す。
   Rust 側 decoder はすべて RGBA を直接出して csx の post-swap と等価にする。
2. **`useVertexColor` の alpha pre-mul**: `gx.frag` で
   `fragColor.rgb *= vc.rgb * vc.aaa`。`colors` は素直に出すが、Blender 側で
   `(a,a,a)` × `vc.rgb` の MULTIPLY が要る (= Rust 側で何もしない、JSON shape
   そのままで OK)。
3. **`TObj.Blending` を必ず出す**: BLEND ColorOp で `mix(diff, tex,
   blending)`。MR_highway は ほぼ `Blending=1.0` で texture そのまま。
   `0.5` 固定にすると白被りバグ。csx と同じく float 値そのまま JSON に出す。
4. **`MObj.RenderFlags` は flags 文字列 + raw 両方**を出す (Blender addon は
   `'XLU' in flags` 等の string 検査をしている)。
5. **alias root の identity 管理**: §3 のとおり `Rc` identity ベース、byte 一致
   ではない。`HashSet<*const HsdStruct>` で walk visited を管理。
6. **`scene_data` 経由 walk + 残り root 順走**: csx の順序で処理しないと
   `joint_aliases` の対応関係が崩れる (§2)。
7. **GLSL shader が「何が見えるべきか」の一次資料**: `HSDRawViewer/Shader/gx*.frag`。
   合成式の確認はここを開く。HSDLib のソースだけだと material の合成式が
   分からない (rendering layer が分離)。
8. **PNG metadata**: `image` クレートのデフォは PNG に gAMA/pHYs を書き込む
   ことがある。csx 出力と byte 一致させるには encoder を最小構成 (RGBA8 raw,
   no ancillary chunks) で起こす。Phase 2 で確認、必要なら自前 PNG writer
   を持つか ImageSharp 出力を観察してチャンク構成を合わせる。
9. **C# `double.ToString` vs Rust `f64::to_string`**: parity test は **数値
   比較**で行う、文字列比較は禁止 (§4)。
10. **HSDLib `Dictionary<int,...>` は insertion 順**、Rust の `HashMap` は
    非決定的。reloc table 出力など順序が effective に影響する経路は
    `BTreeMap` または `IndexMap` を使う。

## 8. 内部 model と accessor accessor 表

実装で必要になる主要 struct の field 一覧 (ground truth は HSDLib のソース):

### HSD_SOBJ (`Common/HSD_SOBJ.cs`、TrimmedSize 0x10)
- 0x00 ref `HSDNullPointerArrayAccessor<HSD_JOBJDesc>` JOBJDescs
- 0x04 ref Camera
- 0x08 ref Lights
- 0x0C ref Fog

`HSDNullPointerArrayAccessor<T>` は ref のリストを **NULL ポインタ終端**で
読む形式。要素数は array 内の null 検出で決まる。

### HSD_JOBJDesc (`Common/HSD_SOBJ.cs:35`、TrimmedSize 0x10)
- 0x00 ref HSD_JOBJ RootJoint
- 0x04..0x0C anim (Phase スコープ外)

### HSD_JOBJ (`Common/HSD_JOBJ.cs`、TrimmedSize 0x40)
- 0x00 string ClassName (ref)
- 0x04 i32 Flags (JOBJ_FLAG bitmask、OPA/XLU/SPLINE/PTCL 等)
- 0x08 ref HSD_JOBJ Child
- 0x0C ref HSD_JOBJ Next
- 0x10 ref union { HSD_DOBJ Dobj | HSD_Spline Spline | HSD_ParticleJoint ParticleJoint }
       Flags & (SPLINE|PTCL) で切り替え。コース mesh は通常 Dobj。
- 0x14..0x28 float RX RY RZ SX SY SZ TX TY TZ (各 4 byte)
- 0x38 ref HSD_Matrix4x3 InverseWorldTransform (基本 unset)
- 0x3C ref HSD_ROBJ ROBJ (constraint 系、コースでは無し)

### HSD_DOBJ (`Common/HSD_DOBJ.cs`、TrimmedSize 0x10)
- 0x00 string ClassName
- 0x04 ref HSD_DOBJ Next
- 0x08 ref HSD_MOBJ Mobj
- 0x0C ref HSD_POBJ Pobj

### HSD_MOBJ (`Common/HSD_MOBJ.cs`、TrimmedSize 0x18)
- 0x00 string ClassName
- 0x04 i32 RenderFlags (RENDER_MODE bitmask)
- 0x08 ref HSD_TOBJ Textures (chain start, Next で繋がる)
- 0x0C ref HSD_Material
- 0x14 ref HSD_PEDesc

### HSD_Material (`Common/HSD_MOBJ.cs:101`、TrimmedSize 0x14)
- 0x00 byte AMB_R/G/B/A (4 byte)
- 0x04 byte DIF_R/G/B/A
- 0x08 byte SPC_R/G/B/A
- 0x0C float Alpha
- 0x10 float Shininess

### HSD_TOBJ (`Common/HSD_TOBJ.cs`、TrimmedSize 0x5C)

詳細は `HSD_TOBJ.cs:90-220` 参照。Phase 2/3 で読む key field のみ抜粋:

- 0x00 string ClassName, 0x04 ref Next
- 0x08 i32 TexMapID (GX_TEXMAP0..7)
- 0x0C i32 GXTexGenSrc
- 0x10..0x30 RX..TZ (UV transform)
- 0x34 i32 WrapS (CLAMP=0, REPEAT=1, MIRROR=2)
- 0x38 i32 WrapT
- 0x3C u8 RepeatS, 0x3D u8 RepeatT
- 0x40 i32 Flags (TOBJ_FLAGS — COLORMAP/ALPHAMAP は flags の中、§下表)
- 0x44 float Blending
- 0x48 i32 MagFilter (GXTexFilter)
- 0x4C ref HSD_Image
- 0x50 ref HSD_Tlut (palette、CI* 用)
- 0x54 ref HSD_TOBJ_LOD
- 0x58 ref HSD_TOBJ_TEV

`TOBJ_FLAGS` の `ColorOperation` は `(Flags >> 16) & 0xF`、`AlphaOperation` は
`(Flags >> 20) & 0xF`、両方 `COLORMAP` / `ALPHAMAP` enum 値 (NONE=0,
ALPHA_MASK=1, RGB_MASK=2, BLEND=3, MODULATE=4, REPLACE=5, PASS=6, ADD=7, SUB=8)。
csx は `.ToString()` で enum 名を取り、`MODULATE`/`REPLACE`/`BLEND`/`RGB_MASK`/`ADD`
等を JSON に出している (`COLORMAP_*` prefix なし)。

### HSD_Image (`Common/HSD_TOBJ.cs:341`、TrimmedSize 0x18)
- 0x00 ref byte[] ImageData (= raw GX texture buffer; この struct は buffer
       (References.Count==0) 扱いで `IsBufferAligned=true`)
- 0x04 i16 Width, 0x06 i16 Height
- 0x08 i32 Format (GXTexFmt)
- 0x0C i32 MipMap
- 0x10 float MinLOD, 0x14 float MaxLOD

### HSD_Tlut (`Common/HSD_TOBJ.cs:368`、TrimmedSize 0x20)
- 0x00 ref byte[] TlutData (palette buffer)
- 0x04 i32 Format (GXTlutFmt: IA8=0, RGB565=1, RGB5A3=2)
- 0x08 i32 GXTlut
- 0x0C i16 ColorCount

## 9. テスト用 .dat の準備計画

handoff §テストデータより、`tests/data/synthetic_*.dat` を repo commit。
Phase 4 で本格対応するが Phase 0 で目処を決めておく:

- **synthetic_minimal.dat**: scene_data only (空 SOBJ, JOBJDescs 1 個に
  Identity transform の単一 JObj)。reader と writer の最小 round-trip。
  数百 byte。
- **synthetic_one_mesh.dat**: 上 + 三角形 1 枚 (DObj+POBJ)、CMP テクスチャ
  4×4 1 枚、material 1 個。各 accessor の最小 path 確認。数 KB。
- **synthetic_alias.dat**: 上 + alias root を持つ第二 root (`scene_data` の
  child JObj に `alpha_joint` という別 root 名を貼る)。alias round-trip 確認。

これらは初版を hand-craft するか、Rust writer で生成して HSDLib 側で読んで
OK 出るか確認するか、 vanilla の小さい subtree を切り出してくるかの 3 択。
**hand-craft (or Rust writer 出力) を `dotnet-script` 経由 HSDLib で open
してエラーが出ない**ことを minimum gate にする。

vanilla MKGP2 corpus (Phase 4 の `--ignored` 対象) の最低カバー:

- `MR_highway_short_A.dat` (約 2.7 MB、CMP/RGB5A3 大半、alias root 13 個)
- `MR_highway_long_A.dat`
- `mc_jungle` (= `DK_jungle_short_a.dat` 想定、handoff の名前と若干食い違うが
  実在 path 優先)
- `mc_kingdom` / `mc_palace` / `st_pyramid` … handoff 記載の 6 ファイル名は
  実在 file 名と差異がある可能性。Phase 4 で実 file list と照合し、
  **実在 6 つ**を最終選定する (`mr_highway_short/long_A`, `dk_jungle_short_a`,
  `dk_jungle_long_a`, `wc_dcity_short_A`, `pc_land_short_a` を候補にする)。

## 10. Phase 1 入口時点の TODO

- workspace 既存の `hsdraw-core/src/lib.rs` の `add(2,2)` dummy を消し、
  `pub mod dat;` `pub mod hsd_struct;` から始める。
- `hsdraw-cli` は最初は header dump (`fsize, relocCount, rootCount, refCount,
  version, root names`) を stdout に出すところから。
- 依存追加: `byteorder` か `binread`/`binrw`、`thiserror`、`indexmap`。
  `cargo add` で最新を入れる。pyo3 はすでに 0.28.3 入り。
- error 型は Phase 1 で `HsdError` を作って `Result<T> = Result<T, HsdError>`
  で統一。

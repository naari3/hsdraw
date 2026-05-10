# HSD TObj runtime pipeline — identity-skip 調査レポート

**観察源** (= 一次資料):
- `Ploaj/HSDLib` @ d1410f4 — `HSDRawViewer` の OpenGL ビューア。
- `doldecomp/melee` (sysdolphin/baselib) — `tobj.c` / `tobj.h` の HAL HSD
  ランタイム。MKGP2 (Triforce Mario Kart GP 2) と同じ sysdolphin (HSD)
  系列の C ソース。MKGP2 自体は decomp が無いが、libhsd / sysdolphin の
  挙動はここから推測できる。

スコープ: 「scale=(1,1,1) rot=0 trans=0 の identity TObj transform で、
runtime が matrix load を skip しているか / どの GX matrix slot に load
されるか」を確認する。

---

## 結論サマリ

| 経路 | identity-skip 有無 | 影響 |
|---|---|---|
| **HSDRawViewer (GL)** | **無し** — `_shader.SetMatrix4x4(... transform)` を毎回呼び出す。fragment shader は `(transform * vec4(uv,0,1))` を常に実行 | ビューアーで問題なく描画される |
| **sysdolphin baselib (GC HW)** | **無し** — `MakeTextureMtx()` は条件付き分岐なしで TRS を構築、`TObjSetupMtx()` は無条件で `GXLoadTexMtxImm()` を呼ぶ | GC で identity TRS を skip する経路は存在しない |
| **GC HW の default UV パス** | (skip ではないが) **TRS 行列が事実上使われない** | TObj.scale/rot/trans を変更しても UV は変わらない可能性が高い ★ |

★ がユーザーが観察している flat-color 現象の根本要因の候補だが、これは
"identity skip" ではなく "matrix bypass via `mtx=GX_IDENTITY`" の話。後述。

---

## 1. HSDRawViewer (GL) renderer

### 1.1 行列構築 — `LiveTObj.MakeMatrix`

`HSDRawViewer/Rendering/Models/LiveTObj.cs` :

```csharp
public Matrix4 MakeMatrix() {
    Matrix4 trans = Matrix4.CreateTranslation(
        -TX,
        -(TY + (TOBJ.WrapT == GXWrapMode.MIRROR ? 1f / (TOBJ.RepeatT / SY) : 0f)),
        TZ);
    Matrix4 rot = Math3D.CreateMatrix4FromEuler(RX, RY, -RZ);
    Matrix4 scale = Matrix4.CreateScale(
        Math.Abs(SX) < Single.Epsilon ? 0 : TOBJ.RepeatS / SX,
        Math.Abs(SY) < Single.Epsilon ? 0 : TOBJ.RepeatT / SY,
        SZ);
    return trans * rot * scale;
}
```

`SX=SY=SZ=1`, `RepeatS=RepeatT=1`, `RX=RY=RZ=0`, `TX=TY=TZ=0` を入れると:
- trans = identity
- rot = identity
- scale = (1/1, 1/1, 1) = identity
- 結果 = identity matrix

つまり identity transform はそのまま identity matrix に落ちる。**短絡する
判定はない**。

### 1.2 行列 upload — `RenderJObj.cs:816`

```csharp
_shader.SetMatrix4x4($"TEX[{i}].transform", ref transform);
```

無条件 upload。シェーダ側 (`gx_lightmap.frag:139`):

```glsl
vec4 coordtransform = (tex.transform * vec4(coords.x, coords.y, 0, 1));
```

無条件適用。identity matrix × UV = UV。OK。

→ **HSDRawViewer GL ビューアーは正しく描画する**。

---

## 2. sysdolphin baselib (GC HW)

`doldecomp/melee/src/sysdolphin/baselib/tobj.c` の関数群を順に追う。

### 2.1 `MakeTextureMtx(HSD_TObj* tobj)` (l. 358 周辺)

```c
scale.x = __fabsf(tobj->scale.x) < FLT_EPSILON
              ? 0.0F
              : (f32) tobj->repeat_s / tobj->scale.x;
scale.y = __fabsf(tobj->scale.y) < FLT_EPSILON
              ? 0.0F
              : (f32) tobj->repeat_t / tobj->scale.y;
scale.z = tobj->scale.z;
rot.x = tobj->rotate.x;
rot.y = tobj->rotate.y;
rot.z = -tobj->rotate.z;
trans.x = -tobj->translate.x;
trans.y = -(tobj->translate.y + (tobj->wrap_t == GX_MIRROR
              ? 1.0F / (tobj->repeat_t / tobj->scale.y)
              : 0.0F));
trans.z = tobj->translate.z;

PSMTXTrans(tobj->mtx, trans.x, trans.y, trans.z);
HSD_MkRotationMtx(m, (Vec3*) &rot);
MTXConcat(m, tobj->mtx, tobj->mtx);
MTXScale(m, scale.x, scale.y, scale.z);
MTXConcat(m, tobj->mtx, tobj->mtx);
```

HSDRawViewer の `MakeMatrix` と完全に同一の計算式。**identity 判定無し**。

### 2.2 `TObjSetupMtx(HSD_TObj* tobj)` (default ブランチ)

```c
default:
    if (tobj_bump(tobj)) {
        GXLoadTexMtxImm(tobj->mtx, tobj->mtxid, GX_MTX2x4);
    } else {
        GXLoadTexMtxImm(tobj->mtx, tobj->mtxid, GX_MTX3x4);
    }
    break;
```

無条件 `GXLoadTexMtxImm`。**identity short-circuit 無し**。

### 2.3 `setupTextureCoordGen(HSD_TObj* tobj)` (default ブランチ)

```c
default:
    if (tobj_bump(tobj)) {
        GXSetTexCoordGen(tobj->coord, GX_TG_MTX2x4, tobj->src,
                         tobj->mtxid);
    } else {
        GXSetTexCoordGen2(tobj->coord, GX_TG_MTX2x4, tobj->src,
                          GX_IDENTITY, GX_DISABLE, tobj->mtxid);
    }
```

★ **重要観察**: 非 bump の default UV パスでは
`GXSetTexCoordGen2(coord, GX_TG_MTX2x4, src, mtx=GX_IDENTITY, normalize=GX_DISABLE, postmtx=tobj->mtxid)`
を呼ぶ。

GX hardware は `GXSetTexCoordGen2` の 4 番目引数 (`mtx`) で TexCoord 生成
時に乗算する行列を指定する。ここで `GX_IDENTITY` 固定。`tobj->mtxid` は
**post-transform matrix slot ID** (6 番目の引数) として渡されるが、
`normalize=GX_DISABLE` なので **post-transform 段階自体が無効化** され、
post-mtx は適用されない。

つまり default UV パスでは hardware が
`out_uv = src_attribute × IDENTITY = src_attribute` を吐くだけで、
TObj の TRS 行列 (`tobj->mtx`) は計算・load されても**UV 変換に使われない**。

### 2.4 `tobj->mtxid` の値 (l. 1096)

```c
if (texmap_no < limit) {
    tobj->id = HSD_Index2TexMap(texmap_no++);
    tobj->mtxid = HSD_TexMapID2PTTexMtx(tobj->id);
    ...
}
```

`HSD_TexMapID2PTTexMtx`:
```c
GX_TEXMAP0 → GX_PTTEXMTX0
GX_TEXMAP1 → GX_PTTEXMTX1
... (TEXMAP7 → PTTEXMTX7)
```

つまり `tobj->mtxid` は post-transform slot 0..7 に動的割り当てされる。
**`GX_IDENTITY (60)` ではない**。`GXLoadTexMtxImm(tobj->mtx, GX_PTTEXMTX0, ...)`
は post-transform slot 0 に書き込むが、`normalize=GX_DISABLE` なので
hardware は読みに来ない。

---

## 3. ユーザー仮説への影響

### 仮説 1: 「identity TObj transform で matrix load が skip される」

→ **不成立**。HSDLib viewer も sysdolphin baselib も identity-skip ロジックは
持たない。matrix は無条件に構築・load される。

### 仮説 2: 「TEXMAP0 → TEX0_MTX[60]/IDENTITY slot に load されている」

→ **不成立**。`tobj->mtxid = HSD_TexMapID2PTTexMtx(tobj->id)` で
`GX_PTTEXMTX0..7` のどれかに割り当てられる。`GX_IDENTITY` slot に書き込
まれることはない。

### 仮説 3: 「default UV path で TObj transform は事実上使われない」

→ **新たな観察**: `GXSetTexCoordGen2(coord, MTX2x4, src, GX_IDENTITY, DISABLE, mtxid)`
で `mtx=GX_IDENTITY` 固定。`normalize=DISABLE` なので post-mtx 無効。
**TObj.scale/rotate/translate は default UV path では UV に影響しない**。

これが「user の TObj scale/rot/trans を変えても flat-color が変わらない」
観察と整合する。ただし flat-color の **原因** ではない (TObj transform を
弄って改善する方法は無い、というだけ)。

---

## 4. 残課題: flat-color の真の原因

flat color = checker 2 色の算術平均という観察は、上記の TObj 経路以外に
原因があると思われる。候補:

| 候補 | 検証方法 |
|---|---|
| MagFilter / MinFilter のデフォルト | `tobj->lod == NULL` のとき GX SDK は内部既定値を使う。`tobj.set_lod(min_filter=GX_NEAR, ...)` で明示することで切り分け可能 (本 commit で API 追加済) |
| TEX0 attribute の dequantize / GX_VTXFMT 設定 | dolphin の GX_TEX0 設定を trace してみるか、vanilla `inu_aliased.dat` の POBJ 属性表 (offset 0x08 の attr-table) と user の出力 .dat の attr-table を byte-diff |
| `tobj->coord` (GXTexCoordID slot) の衝突 | 同一 TObj chain で複数 TObj が同じ coord slot に書く設定になっていないか確認 |
| MKGP2 固有の POBJ.flags=0x8000 (vanilla 94-97% が使用) の semantics | doldecomp 圏外 — MKGP2 ELF の POBJ display path を Ghidra で確認するしかない |
| Vertex DL の `0x90 (TRIANGLES)` count フィールド or matrix-index 列 | dolphin Fifo log + GX command stream dump で原寸サンプル取得 |

`set_lod()` API (本 commit) は最低限の切り分けに使える。具体的には:

```python
tobj.set_lod(
    min_filter=0,    # GX_NEAR — テクセル単位サンプリング
    bias=0.0,
    bias_clamp=False,
    enable_edge_lod=False,
    anisotropy=0,    # GX_ANISO_1 — aniso 無効
)
```

を貼ったうえで実機 / dolphin で flat-color が解消するなら、原因は
GX hardware の min_filter 既定値 / aniso 既定値 だった、と切り分けられる。

---

## 5. 補足: TexGenSrc → 入力 attribute の対応

`GXSetTexCoordGen2(coord, GX_TG_MTX2x4, tobj->src, ...)` の 3 番目引数
`tobj->src` は HSD_TObj 0x0C の `GXTexGenSrc` 値そのまま。

| `GXTexGenSrc` | hardware 入力 |
|---|---|
| `GX_TG_POS (0)` | world-space position (XY plane) |
| `GX_TG_TEX0 (4)` | POBJ attribute table の `GX_VA_TEX0` フィールド |
| `GX_TG_TEX1..7` | 同 TEX1..7 |
| `GX_TG_NRM (1)` | world-space normal (XY plane) |

ユーザー設定の `GXTexGenSrc=GX_TG_TEX0` は正しい。POBJ の `GX_VA_TEX0`
attribute がそのまま入力 UV になる。ただし上記 §2.3 で確認した通り、
hardware はこれを **identity 行列で乗算** するだけなので、入力 UV =
出力 UV。POBJ の TEX0 buffer に正しい UV が書かれていれば (= ユーザー
の確認 1 の通り) hardware は正しい UV を引き出す。

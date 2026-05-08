# Phase 6: maturin wheel + Blender addon vendor

handoff の最終フェーズ。Phase 5 までで `hsdraw-py` (PyO3 binding) + Writer
が動く状態。あとは「Blender 内蔵 Python から `import hsdraw` で動く zip」を
出すだけ。

## CI ワークフロー

`.github/workflows/test.yml` — push / PR 毎の sanity gate

- **cargo-test (linux/macos/windows)**: `cargo test --workspace --no-fail-fast`。
  vanilla corpus は `MKGP2_FILES_DIR` env 不在時 skip するので CI に
  ROM 持ち込み不要。`tests/data/synthetic_minimal.dat` 経由で reader+writer
  の両方が gate される
- **wheel-smoke (linux/macos/windows)**: `maturin build --release` → `pip
  install` → `python -c "import hsdraw"`。ABI3 wheel が壊れていないか確認

`.github/workflows/wheels.yml` — リリース時 + 手動 dispatch

- **build-wheels** (5 マトリクス):
  - `linux-x86_64` (manylinux2014, zig クロスコンパイル)
  - `linux-aarch64` (同上、native arm64 runner なしで Linux x86_64 から zig)
  - `macos-x86_64` (macos-latest 上で `--target x86_64-apple-darwin`)
  - `macos-arm64` (同 runner で arm64 ターゲット)
  - `windows-x86_64`
  - すべて `pyo3 0.28.3 + abi3-py37` features なので Python 3.7+ で動く
    1 wheel/プラットフォーム
- **bundle**: 5 platform を `hsdraw-wheels-bundle` に集約。Blender addon
  リリース時はこの artifact を zip に取り込む

`actions/checkout@v4` / `actions/setup-python@v5` / `actions/upload-artifact@v4`
で固定。`Swatinem/rust-cache@v2` で `target/` キャッシュ。

## Blender addon vendor 手順

Blender 4.x の bundled CPython は 3.11 系。`hsdraw-*-cp37-abi3-*.whl` は
3.7+ ABI なので bundled Python でそのまま動く。

addon の zip 内構造 (mkgp2-patch 側に commit する想定):

```
mkgp2_patch_addon/
  __init__.py           # bl_info + sys.path セットアップ
  vendor/
    linux_x86_64/
      hsdraw/             # 解凍した wheel の hsdraw パッケージ
      hsdraw-0.0.1.dist-info/
    linux_aarch64/
    macos_x86_64/
    macos_arm64/
    windows_x86_64/
  ...
```

`__init__.py` 冒頭:

```python
import os, sys, platform

_arch = platform.machine().lower()
_sys = sys.platform
if _sys.startswith("linux"):
    _platform_dir = "linux_aarch64" if _arch in ("aarch64", "arm64") else "linux_x86_64"
elif _sys == "darwin":
    _platform_dir = "macos_arm64" if _arch in ("arm64", "aarch64") else "macos_x86_64"
elif _sys == "win32":
    _platform_dir = "windows_x86_64"
else:
    raise ImportError(f"hsdraw addon: unsupported platform {_sys}/{_arch}")

_vendor = os.path.join(os.path.dirname(__file__), "vendor", _platform_dir)
if _vendor not in sys.path:
    sys.path.insert(0, _vendor)

import hsdraw  # noqa: E402
```

wheel の中身は `unzip hsdraw-*.whl -d vendor/<platform>/` で展開できる
(wheel = ZIP file)。これを `bundle_addon.sh` のような script で:

```bash
# Phase 6 完成基準: 6 platform 分の wheel を artifact から取得して展開
mkdir -p mkgp2_patch_addon/vendor
for whl in hsdraw-wheels-bundle/*.whl; do
    case "$whl" in
        *manylinux*x86_64*) dst=linux_x86_64 ;;
        *manylinux*aarch64*) dst=linux_aarch64 ;;
        *macosx*x86_64*) dst=macos_x86_64 ;;
        *macosx*arm64*) dst=macos_arm64 ;;
        *win_amd64*) dst=windows_x86_64 ;;
        *) continue ;;
    esac
    unzip -q -o "$whl" -d "mkgp2_patch_addon/vendor/$dst"
done
```

## 完了確認チェックリスト

- [x] `.github/workflows/test.yml` で 3 OS × cargo test green
- [x] `.github/workflows/wheels.yml` で 5 platform wheel build (CI で push/dispatch)
- [x] `pyproject.toml` + `maturin develop` でローカル Python に install できる
- [ ] mkgp2-patch addon に上記 vendor 手順を組み込んで vanilla 6 コース
      import 成功 (このリポジトリ範囲外、addon 側の作業)
- [ ] PyPI 公開 / 内部配布 (任意、addon zip に vendor すれば PyPI なしでも
      Blender ユーザは無依存で動く)

## 既知の TODO (Phase 6 範囲外)

- `windows-arm64`, `linux-musl` wheel は需要が出るまで未対応
- 個別 vendor の手間を減らすために `bundle_addon.sh` 自体を mkgp2-patch
  CI に持ち込む (cross-repo artifact 取得)

# preset 内核（开箱即用）

`kernel_preset = "mainline"` 让 virtuoso 使用**官方预编的 mainline mini 内核**
——不装内核树也能 `test`。它解决两件事：

1. **开箱即用**：装好 virtuoso → `fetch` → `test`，三步出 verdict；
2. **托管 CI 的装置 E2E**：GitHub 托管 runner 没有内核树与 KVM，mini 内核
   （恒 TCG 引导）让装置自身的引导链路（init → busybox → testcases →
   judge）每次 push 都被验证。

**定位边界**：preset 内核只承担「装置 E2E」，不是被测内核行为的权威。
openEuler 特有行为（怪癖表：空 `/dev` 回退 mknod、sysfs dev 属性为空、
virtio/ext4 `=m` 进 initramfs 等）在 mainline 上走不到，那些场景仍以
自建 openEuler runner 的 `kernel-e2e` 为准。

## 供给链

```
kernel.org mainline release（约每 9-10 周）
  → kernel-release.yml（每周一检查，cron）
      → infra/kernel/build-preset.sh：三架构 defconfig + fragment.config
        （全 =y 零模块）→ Image-<arch> / config-<arch> / SHA256SUMS
      → GitHub Release（tag kernel-v<版本>）
      → bump PR：infra/kernel/pin 推进到新版本（合并即采用）
  → virtuoso fetch：下载到 target/kernel/preset/（SHA256 校验）
  → test：kernel_preset 激活时内核镜像取 fetch 缓存
```

- **不自动追新**：新版本经 bump PR 进入，CI 的 `E2E (preset kernel, TCG)`
  先在新 pin 上跑绿才能合并——内核回归不会被静默采信；
- 版本解析顺序：`fetch --version` 显式指定 > `infra/kernel/pin` 钉定 >
  最新已发布 release；
- release 仓库缺省解析 git origin（GitHub），可用 `KERNEL_RELEASE_REPO`
  显式指定（fork / 私有镜像场景）。

## 使用

```bash
# virtuoso.toml（或任何命令前 export KERNEL_PRESET=mainline）
kernel_preset = "mainline"

virtuoso fetch                # 三架构全量（约百 MB 级）
virtuoso fetch --arch arm64   # 只拉一架构
virtuoso fetch --version 6.12.8
virtuoso verify               # 源码树检查自动降级为 preset 语义
virtuoso test --timeout 300   # 恒 TCG（托管 runner 无 KVM 同款路径）
```

preset 激活时的语义变化（与源码树路径的差异全在这里）：

| 维度 | 源码树路径（KERNEL_PATH） | preset 路径 |
|---|---|---|
| 内核镜像 | 树内 `arch/<a>/boot/<Image>` | `target/kernel/preset/Image-<arch>` |
| 模块供给 | 树内 `.ko` 拷入 rootfs（缺则构建报错） | 全 `=y` 内建，`.ko` 查找整体跳过 |
| verify 检查 | KERNEL_PATH / Kernel source FAIL 门 | 单一 Info 行（`Kernel preset: mainline …`） |
| init 引导日志 | insmod openEuler `=m` 模块 | 模块清单为空占位，零 insmod 噪音 |
| `suggest` 内核 diff | 对内核树 `git diff` | 需显式 `--diff`（无树可 diff） |

## 自己构建 preset 内核

`infra/kernel/build-preset.sh` 可脱离 CI 本地运行（发布同款资产）：

```bash
sudo apt install gcc-aarch64-linux-gnu gcc-riscv64-linux-gnu
bash infra/kernel/build-preset.sh 6.12.8 out/
# out/: Image-{arm64,x86_64,riscv64} + config-* + SHA256SUMS
```

`fragment.config` 是三架构共用的 merge fragment（架构特有项由 olddefconfig
自然丢弃）：virtio 设备面、ext4、串口 console（PL011/8250）、静态 ELF 执行
面，全部内建。配置清单见
[kernel-release.yml](../.github/workflows/kernel-release.yml) 与
[infra/kernel/](../infra/kernel/)。

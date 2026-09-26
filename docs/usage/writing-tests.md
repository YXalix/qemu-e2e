# 编写测试用例

`init` 自动发现 rootfs `/tests/` 下的所有可执行文件（含一层测试组子目录），
**新增用例零接线**。两条硬规则：

- 测试必须**静态链接**（VM 内无动态加载器；musl 目标 crt-static 是
  rustc 缺省，workspace 配方已冻结，写用例不用管）；
- **禁止用 `|| true` 掩盖失败**。

用例统一走一个 cargo workspace（`infra/testcases/`）：**coda（std
Rust）是框架与入口，C 测试体经用例 crate 的 build.rs 编入同一静态二进制**
（C over Rust）。协议 v1 对 Rust/C 断言一视同仁——同一串口文本、同一套
计数器（C 宏经 FFI 落回 coda，冻结文本单一来源）。

## 新建用例（复制 test-example）

1. 复制 `infra/testcases/test-example/` 为 `test-<name>/`，改 crate
   `Cargo.toml` 的 `name`。workspace members 是 glob（`test-*`），**不用
   编辑 workspace 文件**。crate 名即二进制名，会出现在串口
   `--- Running: test-<name> ---`。

2. **Rust 测试**：用例 crate 的 `src/main.rs` 在 `TESTS` 注册
   `("名称", fn() -> bool)`，断言用 `coda::check!`（或 `pass!/fail!/skip!`
   手动组合）。main 只有一行——banner、panic hook、C 段调用、计数与退出码
   全部在 `coda::run_and_exit`：

```rust
use coda::{run_and_exit, TestCase};

fn my_feature() -> bool {
    // 通过 syscall / /proc / /sys / /dev 驱动被测内核（std 可用）
    coda::check!(!std::fs::read_to_string("/proc/uptime").is_err(), "…")
}

static TESTS: &[TestCase] = &[("my_feature", my_feature)];

fn main() {
    run_and_exit(TESTS);
}
```

3. **C 测试**：`c/` 下放 `.c` 文件（build.rs 自动收集编译），
   断言用 `coda.h` 的宏（`PASS/FAIL/SKIP/INFO`），在 `run_c_tests` 里
   逐个调用。`c/` 为空也无妨——会自动编入空桩兜住符号：

```c
#include "coda.h"

static void test_my_feature(void)
{
    if (/* 期望条件 */)
        PASS("the thing happened");
    else
        FAIL("expected X, got Y");
}

void run_c_tests(void)
{
    test_my_feature();
}
```

4. 重建并运行：

```bash
virtuoso build && virtuoso test --timeout 30
```

> IDE：仓库自带 `.vscode/settings.json`（rust-analyzer 索引本 workspace）；
> C 侧 clangd 吃各用例 crate 生成的 `compile_commands.json`（跑一次
> `cargo check` 即生成）。

## 断言输出规则

- 测试二进制只输出 `[PASS]` / `[FAIL]` / `[SKIP]` / `[INFO]` 断言行
  （一律走宏，不要直接 `printf`/`println!` 协议前缀）；
- `--- Running:` / `PASSED:` / `FAILED:` / `Test Results:` /
  `TEST_COMPLETE:` 是 **init 的汇编层标记**，测试程序禁止输出；
- 退出码由 coda 汇总（任何 FAIL > 0 → 1），C 段计数经 FFI 同源，
  用例代码不需要自己管理计数器与退出码。

协议文本冻结，逐条语义见[冻结契约](../concepts/contracts.md)。

## 需要内核模块的测试

模块需求写在 `virtuoso.toml` 的 `[tests]` 段（给内核特性写测例的自然
归属，不必挂到无关组件上）：

```toml
[tests]
require = ["overlay", "kvm"]   # "<module> [key=val ...]"，恒 runtime 阶段
```

builder 把 `[tests].require` 并入组件并集生成 rootfs
`/lib/modules/modules.conf`（同名模块组件条目优先，这里只补差集）。
VM 能力类的模块（直通、NUMA、pmem 等）仍走[组件机制](../concepts/components.md)。
迭代中的模块优先 `=m` 而非 `=y`，免内核重建。

## 测试组

`/tests/` 下一级子目录即测试组：组内用例的名字 token 是 `组名/用例名`
（串口与 verdict 里显示为 `--- Running: smoke/test-net ---`），平铺二进制
仍是缺省组。组只是**文件布局维度**，标记协议 v1 文本零变化。

把用例声明进组——写在用例 crate 自己的 `Cargo.toml`（声明跟着用例走，
没有中心清单）：

```toml
[package.metadata.virtuoso]
group = "smoke"
```

- 无声明的 crate 平铺 `/tests/`（向后兼容）；
- 组内执行顺序 = 字典序，组间 = 目录名字典序；
- 汇总行 `Test Results: N/M` 恒为全局口径。

## 向 rootfs 添加文件（rootfs.d）

仓库根 `rootfs.d/` 是用户 drop-in 目录（git 忽略，随用随建）：构建时整树
**增量并入** rootfs（目录级并集），适合测试数据、预置脚本、环境文件。
**只增不覆盖**——任何与 builder 组装产物同路径的文件都是构建期错误，
框架文件（init、模块清单、busybox 树）不可能被遮蔽。缺省无此目录 = 无操作。

放一个脚本用例就是加测试：`rootfs.d/tests/<组>/run-foo.sh`（可执行位），
init 按退出码判 PASSED/FAILED，与编译用例同协议同判定。

## 只跑部分用例（迭代加速）

```bash
virtuoso test --only test-myfeature          # 单个（平铺）
virtuoso test --only smoke/test-net          # 单个（组内）
virtuoso test --only smoke/                  # 整组（尾斜杠）
virtuoso test --only test-a,test-b           # 逗号分隔
```

`--only` 经 kernel cmdline 传给 init，只跑名单内条目；标记
协议 v1 不变（选中的照常吐全套标记）。名单零命中（拼错名字）会直接判失败
——不会出现 0/0 假绿。

# 未验证 InsydeDCHU.dll 只读探测

本分支增加 `--probe-unverified`，用于在 **不加入正式 SHA256 白名单** 的情况下测试厂商 `InsydeDCHU.dll` 是否能完成只读采样。

## 安全边界

`--probe-unverified` 会加载指定 DLL 并解析 `GetDCHU_Data_Buffer`，但不会解析 `SetDCHU_Data`。由该模式创建的 `Insyde` 实例没有写入函数指针；即使后续代码误调用 `write()` 或 `automatic()`，也会直接返回错误，不会调用厂商写接口。

这只能保证 ClevoFanControl 自己不会经由 `SetDCHU_Data` 写入。加载第三方厂商 DLL 本身仍会执行该 DLL 的正常初始化代码。

普通 `--probe`、GUI 和 `--hardware-check` 的 SHA256 白名单行为保持不变；未知 DLL 仍然不能用于写风扇。

## 用法

可以直接把 DLL 的绝对路径放在命令后面，无需先修改配置文件。请使用 Debug 运行，这样控制台能直接看到输出：

```powershell
cargo run -- --probe-unverified "C:\Windows\System32\DriverStore\FileRepository\acpibridge1.inf_amd64_6172acb3e289f51f\InsydeDCHU.dll"
```

如果命令后没有路径，则沿用 `ClevoFanControl.x64.json` 中的 `compatibility.dllPath` / 原有自动查找逻辑。

当前待测试候选：

```text
C:\Windows\System32\DriverStore\FileRepository\acpibridge1.inf_amd64_6172acb3e289f51f\InsydeDCHU.dll
SHA256 430D522C811A5DF1037D07714C47316967C015391985E3F9A0915EEFC46D328F
CLEVO package 6.0.91.1, 2024-08-09

C:\Windows\System32\DriverStore\FileRepository\acpibridge1.inf_amd64_2a0208b3676c8a9c\InsydeDCHU.dll
SHA256 0D6AA0586C628BEA1B6BFE4D5B2941D68D603474338EB0BB79730F2410BF1721
CLEVO package 6.0.46.2, 2023-09-15
```

建议先测试较新的 `430D...`。

## 判断结果

成功输出应该包含机型、DLL 路径、SHA256、`只读探测（未验证 DLL）` 和一个 `Sample`。重点检查：

- 风扇通道数量为 1–3；
- 温度在合理范围内；
- duty 在 0–100%；
- RPM 合理，且连续多次探测时会随实际转速变化。

在只读结果确认正常前，不要把候选 SHA256 加入正式白名单，也不要运行 `--hardware-check`。

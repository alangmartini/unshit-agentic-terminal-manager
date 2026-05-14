# Desktop Interaction Regression

This path is kept as a compatibility entry point.

The canonical framework lives in:

```powershell
tests\windows\desktop-regression
```

Prefer:

```powershell
powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List
```

Existing commands that call `scripts\desktop-regression\run.ps1` still forward
to the canonical runner.

To run every migrated suite as separate sequential invocations:

```powershell
cargo xtask desktop-regression --sequential-isolated
```

or through the compatibility wrapper:

```powershell
powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run-all-sequential.ps1
```

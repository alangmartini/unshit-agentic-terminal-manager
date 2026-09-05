### Fixed

- Windows agent launches through `claude.cmd` / `codex.cmd` (Quick Prompt, crash resume, Flow Explorer) lost their arguments whenever more than one of them needed quotes: the PTY daemon started the batch file directly and `cmd.exe` stripped the first and last quote of the command line, so a prompt with spaces produced `'C:\Users\Alan' is not recognized...` instead of an agent. Batch programs now run as `cmd.exe /d /c call <script> <args>`, which keeps every quoted argument intact.

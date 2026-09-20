"""Test process selection without sending real signals. Run with python3."""
import pathlib
import subprocess
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "kill-all.sh"

# Source the script (its main guard prevents execution), then replace every
# process-facing command. Even a broken selection rule can only log a signal.
HARNESS = r'''
source "$1"
shift
repo_root='/tmp/repo with spaces'
OSTYPE=darwin
pgrep() {
    if [[ "$2" == terminal-manager ]]; then
        printf '%s\n' 101 102 103 104 105 106 107 108
    else
        printf '%s\n' 201
    fi
}
lsof() {
    case "$5" in
        101) printf 'p101\nftxt\nn%s/target/debug/terminal-manager\nftxt\nn/usr/lib/dyld\n' "$repo_root" ;;
        102) printf 'n/Applications/Terminal Manager.app/Contents/MacOS/terminal-manager\n' ;;
        103) printf 'n%s/dist/Terminal Manager.app/Contents/MacOS/terminal-manager\n' "$repo_root" ;;
        104) printf 'n/tmp/other/target/debug/terminal-manager\n' ;;
        105) return 1 ;;
        106) printf 'n%s-other/target/debug/terminal-manager\n' "$repo_root" ;;
        107) printf 'n%s/target-codex/release/terminal-manager\n' "$repo_root" ;;
        108) printf 'n%s/target/debug/different-program\n' "$repo_root" ;;
        201) printf 'n%s/target/release/unshit-ptyd\n' "$repo_root" ;;
    esac
}
kill() { printf 'SIGNAL %s\n' "$*"; }
main "$@"
'''


class KillAllTests(unittest.TestCase):
    def run_script(self, *args, harness=HARNESS):
        return subprocess.run(
            ["/bin/bash", "-c", harness, "test", str(SCRIPT), *args],
            capture_output=True, text=True, check=False,
        )

    def signals(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        return [line for line in result.stdout.splitlines() if line.startswith("SIGNAL")]

    def test_default_selects_only_this_checkout_builds(self):
        result = self.run_script()
        self.assertEqual(self.signals(result), ["SIGNAL -9 101", "SIGNAL -9 107", "SIGNAL -9 201"])
        for pid in (102, 103, 104, 105, 106, 108):
            self.assertIn(f"spared terminal-manager pid={pid}", result.stdout)

    def test_dry_run_never_signals(self):
        result = self.run_script("--dry-run")
        self.assertEqual(self.signals(result), [])
        self.assertEqual(result.stdout.count("would kill"), 3)

    def test_all_is_explicit_opt_in(self):
        result = self.run_script("--all")
        self.assertEqual(len(self.signals(result)), 9)
        result = self.run_script("--all", "--dry-run")
        self.assertEqual(self.signals(result), [])
        self.assertEqual(result.stdout.count("would kill"), 9)

    def test_quiet_preserves_selection(self):
        result = self.run_script("--quiet")
        self.assertEqual(len(self.signals(result)), 3)
        self.assertEqual(len(result.stdout.splitlines()), 3)

    def test_unknown_option_fails_before_signaling(self):
        result = self.run_script("--al")
        self.assertEqual(result.returncode, 2)
        self.assertNotIn("SIGNAL", result.stdout)

    def test_help_does_not_signal(self):
        self.assertEqual(self.signals(self.run_script("--help")), [])

    def test_changed_executable_is_spared(self):
        harness = HARNESS.replace('main "$@"', r'''
pgrep() { [[ "$2" != terminal-manager ]] || echo 101; }
executable_path() {
    if [[ -f "$marker" ]]; then
        echo '/Applications/Terminal Manager.app/Contents/MacOS/terminal-manager'
    else
        touch "$marker"
        echo "$repo_root/target/debug/terminal-manager"
    fi
}
marker="$(mktemp)"
rm "$marker"
trap 'rm -f "$marker"' EXIT
main "$@"
''')
        result = self.run_script(harness=harness)
        self.assertEqual(self.signals(result), [])
        self.assertIn("executable changed", result.stderr)

    def test_linux_uses_proc_executable(self):
        harness = HARNESS.replace('main "$@"', r'''
OSTYPE=linux-gnu
# Only /proc/<pid>/exe is accepted, and lsof must not be used on Linux.
readlink() {
    case "$1" in
        /proc/101/exe) echo "$repo_root/target/debug/terminal-manager" ;;
        /proc/201/exe) echo "$repo_root/target/release/unshit-ptyd" ;;
        *) return 1 ;;
    esac
}
lsof() { echo 'unexpected lsof call' >&2; return 1; }
main "$@"
''')
        self.assertEqual(self.signals(self.run_script(harness=harness)), ["SIGNAL -9 101", "SIGNAL -9 201"])


if __name__ == "__main__":
    unittest.main()

"""Exercise release selection with real annotated/lightweight tags and merges."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('release_refs', Path(__file__).parents[1] / 'ci/release_refs.py')
release_refs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release_refs)


class ReleaseRefsTest(unittest.TestCase):
    def git(self, *args):
        return subprocess.check_output(['git', *args], text=True, stderr=subprocess.DEVNULL).strip()

    def setUp(self):
        self.previous = os.getcwd()
        self.temp = tempfile.TemporaryDirectory()
        os.chdir(self.temp.name)
        self.git('init', '-b', 'main')
        self.git('config', 'user.name', 'Test')
        self.git('config', 'user.email', 'test@example.com')
        self.git('commit', '--allow-empty', '-m', 'initial')
        self.before = self.git('rev-parse', 'HEAD')
        self.git('update-ref', 'refs/remotes/origin/main', self.before)
        self.repo = {'default_branch': 'main'}

    def tearDown(self):
        os.chdir(self.previous)
        self.temp.cleanup()

    def event(self, ref):
        return {'repository': self.repo, 'ref': ref, 'before': self.before, 'after': self.git('rev-parse', 'HEAD')}

    def test_tag_after_merge(self):
        self.git('tag', '-a', 'v1.2.3', '-m', 'release')
        self.assertEqual(release_refs.select('push', self.event('refs/tags/v1.2.3')), [{'ref': 'v1.2.3', 'dry_run': False}])

    def test_tag_before_merge_is_deferred_then_selected(self):
        self.git('checkout', '-b', 'feature/test')
        self.git('commit', '--allow-empty', '-m', 'feature')
        self.git('tag', '-a', 'v1.2.3', '-m', 'release')
        self.assertEqual(release_refs.select('push', self.event('refs/tags/v1.2.3')), [])
        self.git('checkout', 'main')
        self.git('merge', '--no-ff', 'feature/test', '-m', 'merge')
        self.git('update-ref', 'refs/remotes/origin/main', 'HEAD')
        self.assertEqual(release_refs.select('push', self.event('refs/heads/main')), [{'ref': 'v1.2.3', 'dry_run': False}])

    def test_untagged_merge_and_old_tag_do_not_release(self):
        self.git('tag', 'v1.0.0')
        self.git('commit', '--allow-empty', '-m', 'ordinary merge')
        self.assertEqual(release_refs.select('push', self.event('refs/heads/main')), [])

    def test_invalid_tags_and_deletions(self):
        for tag in ['v1.2.3-rc.1', 'v01.2.3', 'v1', 'v1.2.3bad']:
            self.git('tag', tag)
            self.assertEqual(release_refs.select('push', self.event('refs/tags/' + tag)), [])
        self.assertEqual(release_refs.select('push', {'repository': self.repo, 'deleted': True}), [])

    def test_dispatch_allows_main_only_for_dry_run(self):
        event = {'repository': self.repo, 'inputs': {'ref': 'main', 'dry_run': 'true'}}
        self.assertEqual(release_refs.select('workflow_dispatch', event), [{'ref': 'main', 'dry_run': True}])
        event['inputs']['dry_run'] = 'false'
        with self.assertRaises(ValueError):
            release_refs.select('workflow_dispatch', event)

    def test_dispatch_rejects_unmerged_tag(self):
        self.git('checkout', '-b', 'feature/test')
        self.git('commit', '--allow-empty', '-m', 'unmerged')
        self.git('tag', 'v1.2.3')
        with self.assertRaises(ValueError):
            release_refs.select('workflow_dispatch', {'repository': self.repo, 'inputs': {'ref': 'v1.2.3', 'dry_run': 'false'}})


if __name__ == '__main__':
    unittest.main()

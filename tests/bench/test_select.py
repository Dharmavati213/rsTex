#!/usr/bin/env -S python3 -I
"""test_select.py: the rule of select.py holds what tests/bench/README.md says it holds.

    python3 -I tests/bench/test_select.py [-v]

Two groups.  On a census made here, which needs nothing but this checkout: the rule is deterministic, refuses anything of rstex's, leaves out what the oracle fails on and nothing else, and the public part does not depend on
the private part.  On the committed set.json, and set.local.json and the census where this checkout has them: tiers nest, the hold-out touches nothing, each entry is pinned, every stratum is there.  A test that needs a file this
checkout does not have says it was skipped.  Nothing here prints a private id: a failure names a count, or a public id.
"""
import os
import sys

# select.py in this directory is not the standard library's select: subprocess needs that one, so this directory must not be on the path.
sys.path[:] = [p for p in sys.path if os.path.abspath(p or os.curdir) != os.path.dirname(os.path.abspath(__file__))]

import ast  # noqa: E402
import importlib.util  # noqa: E402
import json  # noqa: E402
import random  # noqa: E402
import re  # noqa: E402
import subprocess  # noqa: E402
import tempfile  # noqa: E402
import unittest  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))


def load(name):
    spec = importlib.util.spec_from_file_location('bench_' + name, os.path.join(HERE, name + '.py'))
    mod = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


select = load('select')
edits = load('edits')
SET_JSON = os.path.join(HERE, 'set.json')
SET_LOCAL = os.path.join(HERE, 'set.local.json')
LANDMARKS_LOCAL = os.path.join(HERE, 'landmarks.local.json')
CENSUS = os.path.join(REPO, 'target', 'bs-design', 'census.jsonl')
TREE = os.path.join(REPO, 'target', 'bs-design', 'tree.json')
DIAGRAMS = os.path.join(REPO, 'target', 'bs-design', 'diagrams.jsonl')


def fake_census(seed=1, n=900):
    """A census of made-up documents with every kind, size and feature, in the shape census.py writes.  Ladder documents carry the ids tools/pgo.py trains on, as do a few papers."""
    rnd = random.Random(seed)
    rows = []
    hello = {'id': 'ladder/04-latex-hello', 'population': 'ladder', 'field': 'ladder', 'family': 'ladder', 'instr_total': 1187000000, 'instr_preamble': 631000000, 'mode': 'latex', 'pages': 1, 'passes': 1}
    rows.append(hello)
    fields = ['math.AG', 'math.CT', 'cs.AI', 'cs.PL', 'hep-th', 'stat.ME', 'notes', 'stacks', 'books', 'manuals', 'docs']
    for k in range(n):
        field = rnd.choice(fields)
        if field.startswith(('math.', 'cs.', 'hep-', 'stat.')):
            i, fam = f'arxiv/{field}/{2000 + k}.0000{k}v1', None
        else:
            fam = f'{field}/p{rnd.randrange(12)}'
            i = f'{fam}/d{k}'
        rows.append({'id': i, 'field': field, 'family': fam or i, 'population': 'real', 'pages': rnd.choice([1, 2, 5, 8, 12, 20, 30, 50, 90, 150, 400, 700, 1500]), 'mode': 'latex', 'passes': rnd.randrange(1, 5),
                     'instr_total': rnd.randrange(2, 400) * 100000000, 'instr_preamble': 600000000, 'cpu_passes_pc': [rnd.randrange(500, 9000) * 1.0]})
    for k in range(120):
        rows.append({'id': f'l3build/latex2e/suite{k % 6}/t{k}', 'field': 'l3build', 'family': f'l3build/latex2e/suite{k % 6}', 'population': 'l3build', 'pages': rnd.choice([0, 1]), 'mode': 'latex', 'passes': 1,
                     'instr_total': 900000000, 'instr_preamble': 700000000, 'cpu_passes_pc': [200.0]})
    for r in rows:
        r.update({'private': False, 'class': 'normal', 'judged_failure': False, 'excluded': False, 'verdict': 'substantive', 'source_closure': f'{select.hash_of(r["id"])[:20]}', 'tree_closure': 'tree', 'oracle_fresh': True, 'count_rc': 0,
                  'errors': 0, 'on_pc': True, 'pass_cpu_ms': [1000.0, 900.0]})
        for flag, src, limit in (('tikz', 'src_tikz', 40), ('tikzcd', 'src_tikzcd', 12), ('xy', 'src_xy', 12), ('pgfplots', 'src_axis', 3), ('microtype', None, 0), ('biblatex', None, 0), ('ran_bibtex', None, 0), ('ran_makeindex', 'src_index', 30), ('beamer', None, 0)):
            if rnd.random() < 0.05 and (flag != 'beamer' or r['field'] != 'l3build'):
                r[flag] = True
                if src:
                    r[src] = limit
        if rnd.random() < 0.05:
            r['images'] = 30
        if rnd.random() < 0.05:
            r['src_table_rows'] = 300
        r['edit_kinds'] = list(edits.KINDS)
    return rows


def fake_private(n=40):
    return [{'id': f'local/priv{k % 7}/d{k}', 'field': 'private', 'family': f'local/priv{k % 7}', 'population': 'private', 'private': True, 'pages': 10 + k, 'mode': 'latex', 'passes': 2, 'instr_total': 2000000000 + k, 'instr_preamble': 900000000,
             'class': 'normal', 'judged_failure': False, 'excluded': False, 'verdict': 'substantive', 'source_closure': f'{k:020x}', 'tree_closure': 'tree', 'oracle_fresh': True, 'count_rc': 0, 'errors': 0, 'on_pc': k % 3 > 0, 'pass_cpu_ms': [1000.0]}
            for k in range(n)]


TREE_FAKE = {'oracle_banner': 'test'}


class Rule(unittest.TestCase):
    def test_this_directory_does_not_shadow_the_standard_select(self):
        for name in ('census.py', 'baselines.py', 'select.py'):
            with open(os.path.join(HERE, name)) as f:
                self.assertIn('sys.path[:] = [p for p in sys.path', f.read(), name)
        r = subprocess.run([sys.executable, os.path.join(HERE, 'select.py'), '--help'], capture_output=True, text=True, check=False)
        self.assertEqual(r.returncode, 0, r.stderr[-300:])

    def test_same_bytes_twice(self):
        a, _ = select.build(fake_census(), [], TREE_FAKE, None)
        b, _ = select.build(fake_census(), [], TREE_FAKE, None)
        self.assertEqual(select.dumps(a), select.dumps(b))

    def test_input_order_does_not_matter(self):
        rows = fake_census()
        a, _ = select.build(rows, [], TREE_FAKE, None)
        shuffled = list(rows)
        random.Random(7).shuffle(shuffled)
        b, _ = select.build(shuffled, [], TREE_FAKE, None)
        self.assertEqual(select.dumps(a), select.dumps(b))

    def test_refuses_anything_of_rstex(self):
        for column in ('candidate_ms', 'candidate_cpu_ms', 'speedup', 'outcome', 'levels', 'first_divergence', 'rstex_ms', 'commute_ok'):
            rows = fake_census(n=60)
            rows[5][column] = 1.0
            with self.assertRaises(SystemExit, msg=column):
                select.build(rows, [], TREE_FAKE, None)

    def test_the_program_text_reads_no_column_of_rstex(self):
        """Besides the list that refuses them, no name of a candidate column is in the program: not as a key, not as an attribute, not in a string."""
        with open(os.path.join(HERE, 'select.py')) as f:
            tree = ast.parse(f.read())
        allowed_lines = set()
        for node in tree.body:
            if isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name) and node.targets[0].id in ('FORBIDDEN', 'FORBIDDEN_WORDS'):
                allowed_lines |= set(range(node.lineno, node.end_lineno + 1))
        bad = []
        for node in ast.walk(tree):
            if isinstance(node, ast.Constant) and isinstance(node.value, str) and node.lineno not in allowed_lines:
                if node.value in select.FORBIDDEN:
                    bad.append(node.value)
        self.assertEqual(bad, [])

    def test_census_drops_candidate_columns(self):
        census = load('census')
        record = {'id': 'x', 'mode': 'last-pass', 'source': 's', 'pages': 3, 'oracle_ms': 5.0, 'oracle_class': 'normal', 'candidate_ms': 1.0, 'candidate_cpu_ms': 1.0, 'candidate_rc': 0, 'speedup': 5.0, 'outcome': 'identical', 'levels': {}, 'first_divergence': None}
        kept = census.oracle_columns(record)
        self.assertEqual(sorted(kept), sorted(k for k in record if k in census.ORACLE_COLUMNS))
        self.assertFalse(select.forbidden_in(kept))
        with tempfile.NamedTemporaryFile('w', suffix='.jsonl', delete=False) as f:
            f.write(json.dumps(record) + '\n')
        try:
            read = census.read_records(f.name)
        finally:
            os.unlink(f.name)
        self.assertFalse(select.forbidden_in(read['x']['last-pass']))

    def test_candidate_columns_change_nothing_in_a_census_of_records(self):
        """Two whole-corpus files that differ only in what rstex did give the same census columns."""
        census = load('census')
        base = {'id': 'x', 'mode': 'last-pass', 'source': 's', 'pages': 3, 'oracle_ms': 5.0, 'oracle_class': 'normal', 'oracle_passes': 2, 'tier': 'full'}
        good = dict(base, candidate_ms=1.0, speedup=5.0, outcome='identical')
        worse = dict(base, candidate_ms=900.0, speedup=0.01, outcome='candidate-crash', first_divergence='page 1')
        self.assertEqual(census.oracle_columns(good), census.oracle_columns(worse))

    def test_oracle_failure_stays_out_by_class(self):
        rows = fake_census()
        target = next(r for r in rows if r['population'] == 'real')
        chosen = target['id']
        target['class'] = 'oracle-fails'
        doc, _ = select.build(rows, [], TREE_FAKE, None)
        self.assertNotIn(chosen, {e['id'] for e in doc['entries']})

    def test_a_judged_failure_a_vacuous_entry_and_an_empty_one_stay_out(self):
        rows = fake_census()
        reals = [r for r in rows if r['population'] == 'real']
        reals[0]['judged_failure'] = True
        reals[1]['verdict'] = 'vacuous'
        reals[2]['verdict'] = 'duplicate'
        reals[3]['pages'] = 0
        reals[4]['excluded'] = True
        out = {r['id'] for r in reals[:5]}
        doc, _ = select.build(rows, [], TREE_FAKE, None)
        self.assertFalse(out & {e['id'] for e in doc['entries']})

    def test_a_document_rstex_fails_on_cannot_be_told(self):
        """The rule has no input that says so: a census row has no such column, and the set is what it would be."""
        rows = fake_census()
        doc, _ = select.build(rows, [], TREE_FAKE, None)
        allowed = set()
        for r in rows:
            allowed |= set(r)
        self.assertFalse(select.forbidden_in(dict.fromkeys(allowed)))
        ids = {e['id'] for e in doc['entries']}
        self.assertEqual(ids, {e['id'] for e in select.build(rows, [], TREE_FAKE, None)[0]['entries']})

    def test_public_part_does_not_depend_on_the_private_part(self):
        rows = fake_census()
        priv = fake_private()
        alone, none = select.build(rows, [], TREE_FAKE, None)
        both, part = select.build(rows + priv, [], TREE_FAKE, {priv[0]['id']: 'a landmark'})
        self.assertIsNone(none)
        self.assertIsNotNone(part)
        self.assertEqual(select.dumps(alone), select.dumps(both))
        self.assertFalse([e['id'] for e in both['entries'] if select.is_private_id(e['id'])])
        self.assertTrue(all(select.is_private_id(e['id']) for e in part['entries']))

    def test_tiers_and_holdout_on_the_fake_census(self):
        doc, _ = select.build(fake_census(), [], TREE_FAKE, None)
        check_invariants(self, doc, public=True)

    def test_family_kept_apart_is_whole(self):
        doc, _ = select.build(fake_census(), [], TREE_FAKE, None)
        apart = set(doc['families_kept_apart'])
        self.assertTrue(apart)
        for e in doc['entries']:
            if e['family'] in apart:
                self.assertIn(e['tiers'], (['holdout'], ['reserve']), e['id'])
            else:
                self.assertFalse({'holdout', 'reserve'} & set(e['tiers']), e['id'])

    def test_share_follows_the_population(self):
        """A cell's weight in standard adds up to its mass: what a pooled figure gives it does not depend on how many the floors added."""
        rows = fake_census()
        doc, _ = select.build(rows, [], TREE_FAKE, None)
        total = sum(e.get('weight', 0) for e in doc['entries'] if 'standard' in e['tiers'] and e['kind'] != 'testfile')
        self.assertGreater(total, 0)
        by_cell = {}
        for e in doc['entries']:
            if 'standard' in e['tiers'] and e['kind'] != 'testfile':
                by_cell.setdefault((e['kind'], e['size']), []).append(e['weight'])
        for cell, w in by_cell.items():
            self.assertEqual(len(set(w)), 1, cell)


class Edits(unittest.TestCase):
    MAIN = b'\\documentclass{article}\n\\begin{document}\n' + b''.join(b'one two three four five six seven\n\nalpha beta gamma delta\n\n' for _ in range(30)) + b'\\cite{x}\n\\end{document}\n'

    def test_kinds_apply(self):
        files = [('m', self.MAIN)]
        self.assertEqual(edits.applicable(files, 'm', None, [b'@article{key1,\n title={x}}\n@book{key2, title={y}}\n']), list(edits.KINDS))

    def test_each_edit_changes_what_it_says(self):
        files = [('m', self.MAIN)]
        for k in ('word-start', 'word-middle', 'word-end'):
            name, new = edits.plan(files, 'm', k)
            self.assertEqual(len(new), len(self.MAIN) + len(edits.WORD), k)
        self.assertEqual(edits.plan(files, 'm', 'noop')[1], self.MAIN)
        self.assertIn(b'\\newcommand', edits.plan(files, 'm', 'preamble')[1].split(edits.BEGIN)[0])
        self.assertIn(edits.SENTENCE, edits.plan(files, 'm', 'sentence')[1])
        self.assertIn(b'\\cite{key2}', edits.plan(files, 'm', 'cite', b'\\citation{key1}\n', [b'@article{key1, a=b}\n@book{key2, a=b}'])[1])

    def test_positions_move_through_the_document(self):
        files = [('m', self.MAIN)]
        found = []
        for k in ('word-start', 'word-middle', 'word-end'):
            found.append(edits.plan(files, 'm', k)[1].index(edits.WORD))
        self.assertLess(found[0], found[1])
        self.assertLess(found[1], found[2])

    def test_a_comment_line_and_the_preamble_are_not_places(self):
        text = b'\\documentclass{a}\nthe quick brown fox jumps\n\\begin{document}\n%the quick brown fox jumps\n\\end{document}\n'
        self.assertEqual(edits.word_places([('m', text)], 'm'), [])

    def test_not_applicable_is_none(self):
        text = b'\\documentclass{a}\n\\begin{document}\n\\input{x}\n\\end{document}\n'
        self.assertIsNone(edits.plan([('m', text)], 'm', 'word-middle'))
        self.assertIsNone(edits.plan([('m', text)], 'm', 'cite'))

    def test_multiple_files_in_reading_order(self):
        body = b''.join(b'one two three four five\n\n' for _ in range(40))
        files = [('m', b'\\begin{document}\n\\input{a}\n\\input{b}\n\\end{document}\n'), ('a', body), ('b', body)]
        self.assertEqual(edits.plan(files, 'm', 'word-start')[0], 'a')
        self.assertEqual(edits.plan(files, 'm', 'word-end')[0], 'b')


def check_invariants(test, doc, public):
    entries = doc['entries']
    ids = [e['id'] for e in entries]
    test.assertEqual(len(ids), len(set(ids)))
    tier = {t: {e['id'] for e in entries if t in e['tiers']} for t in ('quick', 'standard', 'full', 'holdout')}
    test.assertLessEqual(tier['quick'], tier['standard'] | tier['quick'])
    test.assertLessEqual(tier['standard'], tier['full'])
    test.assertFalse(tier['holdout'] & tier['standard'])
    test.assertFalse(tier['holdout'] & tier['quick'])
    test.assertFalse(tier['holdout'] & tier['full'])
    for e in entries:
        test.assertTrue(e['source_closure'], e['id'])
        test.assertTrue(e['tree_closure'], e['id'])
        test.assertTrue(e['tiers'], e['id'])
    test.assertTrue(doc['tree'])
    return tier


def present(path):
    return os.path.exists(path)


@unittest.skipUnless(present(SET_JSON), 'no set.json in this checkout')
class Committed(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with open(SET_JSON) as f:
            cls.doc = json.load(f)
        cls.entries = cls.doc['entries']
        cls.tier = {t: {e['id'] for e in cls.entries if t in e['tiers']} for t in ('quick', 'standard', 'full', 'holdout', 'reserve')}

    def test_invariants(self):
        check_invariants(self, self.doc, public=True)

    def test_public_ids_only(self):
        self.assertFalse([i for e in self.entries for i in [e['id']] if select.is_private_id(i)])
        with open(SET_JSON) as f:
            text = f.read()
        self.assertEqual(re.findall(r'"(?:user|local|cd-local)/', text), [])

    def test_local_file_is_ignored(self):
        for name in ('set.local.json', 'landmarks.local.json'):
            r = subprocess.run(['git', 'check-ignore', '-q', os.path.join('tests', 'bench', name)], cwd=REPO, check=False)
            self.assertEqual(r.returncode, 0, name)

    def test_holdout_disjoint_from_training(self):
        all_ids = {e['id'] for e in self.entries}
        training = select.training_ids(all_ids)
        self.assertFalse(self.tier['holdout'] & training)
        trained_families = {e['family'] for e in self.entries if e['id'] in training}
        self.assertFalse({e['id'] for e in self.entries if e['family'] in trained_families and 'holdout' in e['tiers']})

    def test_holdout_disjoint_from_standard_and_quick(self):
        self.assertFalse(self.tier['holdout'] & (self.tier['standard'] | self.tier['quick'] | self.tier['full']))

    def test_no_tool_names_a_holdout_document(self):
        """Any tracked tool or document that names a hold-out id fails here, so that a later tool cannot quietly tune on one."""
        out = subprocess.run(['git', 'grep', '-l', '-F', '-f', '-', '--', 'tools', 'docs', 'bench', 'crates', 'tests/until_stable', 'tests/pages', 'tests/corpus/README.md'], cwd=REPO, input='\n'.join(sorted(self.tier['holdout'] | self.tier['reserve'])), capture_output=True, text=True, check=False)
        self.assertEqual(out.stdout.split(), [])

    def test_public_rows_run_on_the_pc(self):
        self.assertEqual([e['id'] for e in self.entries if not e['on_pc']], [])

    def test_real_size(self):
        self.assertGreaterEqual(len([i for i in self.tier['standard'] if not i.startswith('l3build/')]), 200)
        self.assertGreaterEqual(len([i for i in self.tier['holdout'] if not i.startswith('l3build/')]), 40)

    def test_every_kind_and_size_is_in_standard(self):
        kinds = {e['kind'] for e in self.entries if 'standard' in e['tiers']}
        sizes = {e['size'] for e in self.entries if 'standard' in e['tiers']}
        self.assertEqual(kinds, set(select.KINDS))
        self.assertEqual(sizes, {s for s, _ in select.SIZES})

    def test_every_feature_is_in_standard(self):
        for feature in select.FEATURES:
            have = [e for e in self.entries if feature in e.get('features', ()) and 'standard' in e['tiers'] and e['kind'] != 'testfile']
            full = [e for e in self.entries if feature in e.get('features', ()) and 'full' in e['tiers'] and e['kind'] != 'testfile']
            self.assertGreaterEqual(len(have), min(select.FEATURE_FLOOR, len(full)), feature)
            self.assertTrue(have, feature)

    def test_holdout_has_the_same_strata(self):
        """Every kind, and every size but one that training and landmarks use up, is in the hold-out."""
        kinds = {e['kind'] for e in self.entries if 'holdout' in e['tiers']}
        self.assertLessEqual(set(select.KINDS) - kinds, set())
        sizes = {e['size'] for e in self.entries if 'holdout' in e['tiers']}
        self.assertLessEqual({s for s, _ in select.SIZES} - sizes, set())

    def test_holdout_has_each_feature(self):
        for feature in select.FEATURES:
            self.assertTrue([e for e in self.entries if feature in e.get('features', ()) and 'holdout' in e['tiers']], feature)

    def test_a_tier_looks_like_its_population(self):
        """On what the oracle says: the weighted median of a pass in standard is within a quarter of the population's."""
        real = [e for e in self.entries if e['kind'] != 'testfile' and not e.get('synthetic')]
        sizes = {}
        for e in real:
            sizes[e['family']] = sizes.get(e['family'], 0) + 1
        pop = [(e['minstr'], min(1.0, select.FAMILY_MASS / sizes[e['family']])) for e in real if 'full' in e['tiers']]
        tier = [(e['minstr'], e['weight']) for e in real if 'standard' in e['tiers']]
        a, b = select.weighted_quantiles(pop, (0.5,))[0], select.weighted_quantiles(tier, (0.5,))[0]
        self.assertLess(abs(a - b) / a, 0.25)

    def test_landmarks_are_named_and_marked(self):
        have = {e['id']: e for e in self.entries if e.get('landmark')}
        self.assertEqual(set(have), set(select.LANDMARKS) & {e['id'] for e in self.entries})
        for e in have.values():
            self.assertIn('standard', e['tiers'], e['id'])
        self.assertEqual(self.doc['landmarks_missing'], [])

    def test_training_overlap_is_marked(self):
        all_ids = {e['id'] for e in self.entries}
        training = select.training_ids(all_ids)
        for e in self.entries:
            if e['id'] in training:
                self.assertEqual(e.get('pgo'), 'trained', e['id'])

    def test_estimates_fit_the_tiers(self):
        est = self.doc['estimate']
        self.assertLess(est['quick']['bound']['wall_s'], 15 * 60)
        self.assertLess(est['standard']['bound']['wall_s'], 100 * 60)
        self.assertLess(est['standard']['half']['wall_s'], 75 * 60)
        self.assertLess(est['full']['bound']['wall_s'], 12 * 3600)

    def test_diagrams(self):
        d = self.doc.get('diagrams')
        if not d or not d.get('standard'):
            self.skipTest('no diagrams in the set')
        key = lambda x: x['entry'] + '#' + x['unit']    # noqa: E731
        standard, holdout = {key(x) for x in d['standard']}, {key(x) for x in d['holdout']}
        self.assertFalse(standard & holdout)
        self.assertLessEqual({key(x) for x in d['quick']}, standard)
        self.assertGreaterEqual(len(standard), 150)

    @unittest.skipUnless(present(CENSUS) and present(TREE), 'no census in this checkout')
    def test_committed_file_is_what_the_rule_gives(self):
        rows = select.read_jsonl(CENSUS)
        diagrams = select.read_jsonl(DIAGRAMS) if present(DIAGRAMS) else []
        with open(TREE) as f:
            tree = json.load(f)
        public, _ = select.build(rows, diagrams, tree, None)
        with open(SET_JSON) as f:
            self.assertEqual(select.dumps(public), f.read())


@unittest.skipUnless(present(SET_LOCAL), 'no set.local.json in this checkout')
class Private(unittest.TestCase):
    """The private part, by counts: its failures name no id."""

    @classmethod
    def setUpClass(cls):
        with open(SET_LOCAL) as f:
            cls.doc = json.load(f)
        cls.entries = cls.doc['entries']

    def test_all_private(self):
        self.assertEqual(sum(1 for e in self.entries if not select.is_private_id(e['id'])), 0)

    def test_landmarks_are_in(self):
        with open(LANDMARKS_LOCAL) as f:
            landmarks = json.load(f)
        have = {e['id'] for e in self.entries if e.get('landmark') and 'standard' in e['tiers']}
        self.assertEqual(len(set(landmarks) - have), 0)

    def test_holdout_disjoint(self):
        all_ids = {e['id'] for e in self.entries}
        training = select.training_ids(all_ids)
        holdout = {e['id'] for e in self.entries if 'holdout' in e['tiers']}
        self.assertEqual(len(holdout & training), 0)
        self.assertEqual(len(holdout & {e['id'] for e in self.entries if 'standard' in e['tiers']}), 0)
        trained_families = {e['family'] for e in self.entries if e['id'] in training}
        self.assertEqual(sum(1 for e in self.entries if e['family'] in trained_families and 'holdout' in e['tiers']), 0)

    def test_pinned(self):
        self.assertEqual(sum(1 for e in self.entries if not e['source_closure']), 0)

    def test_holdout_runs_on_the_pc(self):
        self.assertEqual(sum(1 for e in self.entries if 'holdout' in e['tiers'] and not e['on_pc']), 0)


if __name__ == '__main__':
    unittest.main()

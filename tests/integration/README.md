# Integration cases

Small TeX documents, one per feature or per WEB branch, that are compiled by the oracle and by rstex once the engine runs.
Nothing is committed but the documents: the oracle is on the machine, so its log and PDF are made fresh for each compare.
This is where a module's code gets run for the first time when its callees were stubs while it was written, so write the
cases as you port and aim them at the branches you could not run.

    tests/integration/<module>/<name>.tex

The first line says how to run it:

    % mode: ini          pdftex -ini -interaction=nonstopmode NAME.tex
    % mode: ini-etex     pdftex -ini -etex -interaction=nonstopmode *NAME.tex
    % mode: plain        pdftex -interaction=nonstopmode NAME.tex
    % mode: latex        pdflatex -interaction=nonstopmode NAME.tex

The working directory is `tests/integration/`, so `\input prelude` finds `prelude.tex` and `\input <module>/<other>` works.

The second line may set the environment of the run, `% env: NAME=VALUE ...`, with `@WORK@` for the scratch tree and `@PATH@` for the caller's `PATH`.  The cases
`pdffont/pk_mktexpk*` do, and they take a second or two each, since METAFONT runs on both sides.  The bitmaps stay in the scratch tree, not in the user's `TEXMFVAR`.

Compared: the log with banner lines masked, and the PDF's decoded objects.  `\batchmode`, a `\tracingall` or a deliberate
error are all fine and often the point: a log full of error messages and box displays tests more than a clean run does.

`run.py` here does it, and says per case whether the two agree:

    python3 -I tests/integration/run.py --engine target/debug/rstex
    python3 -I tests/integration/run.py --engine target/debug/rstex --only pack --filter limits
    python3 -I tests/integration/run.py --engine target/debug/rstex --first-diff pack/limits
    python3 -I tests/integration/run.py --engine /usr/bin/pdftex --installed-formats     the oracle against itself

Its docstring has the options and what is compared, and the top of the file has every mask with its reason.  It writes only
under `target/integration/`, because a log is engine output.  `PAIRS` and `COMMAND_LINES` in it are what one line of `mode`
cannot say: a `\dump` followed by a run in that format, and the command line itself.

`known_divergences.txt` lists the cases that differ and are accepted, one per line: the case, then a reason a reader can check.
`run.py` lists them apart, with their first difference, and they do not count against the total or the exit status.  Only what
the design makes impossible goes there: PORTING section 15, "Known divergences".  Everything else is a bug.

Rules:

- Synthetic, or using only what TeX Live provides.  Never a personal document of the user.
- A case must run on the oracle to its end, within 10 seconds, and give the same log twice.  Avoid `\pdfcreationdate`, the
  date, the clock and the random seed unless the case sets `\pdfrandomseed` and the harness's `SOURCE_DATE_EPOCH` covers it.
- `ini` modes have no fonts or format loaded: use `\font\x=cmr10` yourself, or `\input plain`.
- No text in `pdf` output is needed for most cases: prefer `\showbox`, `\showlists`, `\tracingoutput`, `\tracingparagraphs`.
- One case exercising many related branches is better than many cases of three lines.

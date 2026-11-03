r"""Hand-written probes for the e-TeX and pdfTeX primitives and the web2c additions.

Same conventions as probes_core.py.  Anything that ends the job goes in FATAL, keyed by case name, each a document of its own.
"""
from probes_core import P, SPECIFIC  # noqa: F401  (SPECIFIC is extended here)

# ---------------------------------------------------------------------------------------------- e-TeX expressions
P('numexpr', r"""
\T{numexpr values}
\V{\the\numexpr 1+2*3\relax}\V{\the\numexpr (1+2)*3\relax}\V{\the\numexpr 1 + 2 * 3 \relax}
\V{\the\numexpr -(1+2)\relax}\V{\the\numexpr --3\relax}\V{\the\numexpr -+-3\relax}
\V{\the\numexpr ((((1))))\relax}\V{\the\numexpr \numexpr 2\relax*3\relax}
\V{\the\numexpr 7*3/2\relax}\V{\the\numexpr 7/2*3\relax}\V{\the\numexpr 7*(3/2)\relax}
\T{numexpr rounding: division rounds half away from zero}
\V{\the\numexpr 7/2\relax}\V{\the\numexpr -7/2\relax}\V{\the\numexpr 7/-2\relax}\V{\the\numexpr -7/-2\relax}
\V{\the\numexpr 5/2\relax}\V{\the\numexpr 3/2\relax}\V{\the\numexpr 1/2\relax}\V{\the\numexpr -1/2\relax}\V{\the\numexpr 1/3\relax}\V{\the\numexpr 2/3\relax}
\V{\the\numexpr 2147483647/2\relax}\V{\the\numexpr -2147483647/2\relax}\V{\the\numexpr 2147483647/2147483647\relax}\V{\the\numexpr 1/2147483647\relax}
\T{numexpr scaling: a*b/c keeps a 64-bit product}
\V{\the\numexpr 100000*100000/100000\relax}
\V{\the\numexpr 2147483647*2147483647/2147483647\relax}
\V{\the\numexpr 2147483647*2/2\relax}\V{\the\numexpr 2147483647*3/4\relax}
\V{\the\numexpr -2147483647*2147483647/2147483647\relax}
\V{\the\numexpr 46341*46341/46341\relax}\V{\the\numexpr 65536*65536/65536\relax}
\V{\the\numexpr 3*4/5*6/7\relax}
\T{numexpr overflow}
\V{\the\numexpr 2147483647\relax}\V{\the\numexpr -2147483647\relax}
\V{\the\numexpr 2147483647+1\relax}
\V{\the\numexpr -2147483647-1\relax}
\V{\the\numexpr -2147483647-2\relax}
\V{\the\numexpr 2147483647+2147483647\relax}
\V{\the\numexpr 100000*100000\relax}
\V{\the\numexpr 46341*46341\relax}\V{\the\numexpr 46340*46340\relax}
\V{\the\numexpr 2147483647*2147483647/1\relax}
\V{\the\numexpr 2147483648\relax}
\V{\the\numexpr 1/0\relax}\V{\the\numexpr 0/0\relax}\V{\the\numexpr -1/0\relax}\V{\the\numexpr 5*3/0\relax}
\V{\the\numexpr 2147483647*(1+1)\relax}\V{\the\numexpr (2147483647+1)*0\relax}
\T{numexpr coercion}
\V{\the\numexpr 1pt\relax}\V{\the\numexpr \dimen0\relax}\V{\the\numexpr \baselineskip\relax}\V{\the\numexpr\thinmuskip\relax}
\V{\the\numexpr `a\relax}\V{\the\numexpr "FF\relax}\V{\the\numexpr '77\relax}
\V{\the\numexpr\count0+\count0\relax}\chardef\c=7 \V{\the\numexpr\c*\c\relax}
\T{numexpr terminators}
\V{\the\numexpr 1 \relax}\V{\the\numexpr 1}\V{\the\numexpr 1 2}\V{\the\numexpr 1+2 3\relax}\V{\the\numexpr 1 x}
\T{numexpr errors}
\V{\the\numexpr\relax}\V{\the\numexpr +\relax}\V{\the\numexpr 1+\relax}\V{\the\numexpr 1*\relax}\V{\the\numexpr 1/\relax}
\V{\the\numexpr (\relax}\V{\the\numexpr (1\relax}\V{\the\numexpr (1+2\relax}\V{\the\numexpr 1)\relax}
\V{\the\numexpr x\relax}\V{\the\numexpr 1+x\relax}\V{\the\numexpr ()\relax}
\V{\the\numexpr 1*1pt\relax}
\T{numexpr uses}
\count1=\numexpr 3*4\relax\showthe\count1 \advance\count1 by\numexpr 1+1\relax\showthe\count1
\ifnum\numexpr 2*3\relax=6 \V{equal}\fi \dimen1=\numexpr 2*3\relax pt\showthe\dimen1
\showthe\numexpr 5\relax
\numexpr 5\relax
\setbox1=\hbox{\numexpr 5\relax}
\numexpr 5\relax=1
\show\numexpr
\def\deep{(((((((((((((((((((((((((((((((((((((((((((((((((1)))))))))))))))))))))))))))))))))))))))))))))))))}\V{\the\numexpr\deep\relax}
""")
P('dimexpr', r"""
\T{dimexpr values}
\V{\the\dimexpr 1pt+2pt\relax}\V{\the\dimexpr 1pt+2pt*3\relax}\V{\the\dimexpr (1pt+2pt)*3\relax}
\V{\the\dimexpr 1pt*3/4\relax}\V{\the\dimexpr 1pt/3*3\relax}\V{\the\dimexpr 1pt/3\relax}\V{\the\dimexpr 2pt/3\relax}
\V{\the\dimexpr -1pt/3\relax}\V{\the\dimexpr 1sp/2\relax}\V{\the\dimexpr 3sp/2\relax}\V{\the\dimexpr -3sp/2\relax}\V{\the\dimexpr 1sp*3/2\relax}
\V{\the\dimexpr (1pt)\relax}\V{\the\dimexpr -(1pt)\relax}\V{\the\dimexpr -(-1pt)*-2\relax}
\V{\the\dimexpr 1in\relax}\V{\the\dimexpr 1cm+1mm+1bp+1pc+1dd+1cc+1sp\relax}\V{\the\dimexpr 1em+1ex\relax}
\T{dimexpr scaling}
\V{\the\dimexpr 10000pt*10000/10000\relax}\V{\the\dimexpr 16383pt*16383/16383\relax}\V{\the\dimexpr 16383pt*2/2\relax}
\V{\the\dimexpr 1pt*2147483647/2147483647\relax}\V{\the\dimexpr 16383.99999pt*3/4\relax}\V{\the\dimexpr 7sp*3/2\relax}
\V{\the\dimexpr 1pt*(1+1)\relax}\V{\the\dimexpr 1pt*\numexpr 2+3\relax\relax}\V{\the\dimexpr 1pt*\count0\relax}\V{\the\dimexpr \baselineskip*2\relax}
\V{\the\dimexpr 1pt*2pt\relax}\V{\the\dimexpr 1pt/1pt\relax}\V{\the\dimexpr 1000pt*1000sp/1000sp\relax}
\T{dimexpr overflow}
\V{\the\dimexpr 16383.99999pt\relax}\V{\the\dimexpr -16383.99999pt\relax}
\V{\the\dimexpr 16383.99999pt+1sp\relax}\V{\the\dimexpr 16383.99999pt+1pt\relax}\V{\the\dimexpr -16383.99999pt-1pt\relax}
\V{\the\dimexpr 16383pt*2\relax}\V{\the\dimexpr 10000pt*10000\relax}\V{\the\dimexpr 16384pt\relax}
\V{\the\dimexpr 1pt/0\relax}\V{\the\dimexpr 0pt/0\relax}\V{\the\dimexpr 1pt*2/0\relax}
\T{dimexpr coercion and errors}
\V{\the\dimexpr 1\relax}\V{\the\dimexpr \count0\relax}\V{\the\dimexpr \skip0\relax}\V{\the\dimexpr 1pt plus 1pt\relax}
\V{\the\dimexpr 1mu\relax}\V{\the\dimexpr\thinmuskip\relax}
\V{\the\dimexpr\relax}\V{\the\dimexpr +\relax}\V{\the\dimexpr 1pt+\relax}\V{\the\dimexpr 1pt*\relax}\V{\the\dimexpr 1pt/\relax}
\V{\the\dimexpr (1pt\relax}\V{\the\dimexpr 1pt)\relax}\V{\the\dimexpr x\relax}\V{\the\dimexpr 1pt+x\relax}\V{\the\dimexpr 2*1pt\relax}
\T{dimexpr uses}
\dimen1=\dimexpr 1pt*3\relax\showthe\dimen1 \advance\dimen1 by\dimexpr 1pt\relax\showthe\dimen1 \count1=\dimexpr 1pt\relax\showthe\count1
\ifdim\dimexpr 1pt+1pt\relax=2pt \V{equal}\fi \skip1=\dimexpr 1pt\relax plus 1pt \showthe\skip1
\showthe\dimexpr 5pt\relax
\dimexpr 5pt\relax
\setbox1=\hbox{\dimexpr 5pt\relax}
\setbox1=\hbox{\kern\dimexpr 1pt\relax\vrule width\dimexpr 2pt\relax\hskip\dimexpr 3pt\relax}\showbox1
\number\dimexpr 1pt\relax\par
""")
P('glueexpr', r"""
\T{glueexpr}
\V{\the\glueexpr 1pt plus 2pt minus 3pt\relax}\V{\the\glueexpr 1pt plus 2fil minus 3fill\relax}
\V{\the\glueexpr 1pt plus 1fil+1pt plus 1fill\relax}\V{\the\glueexpr 1pt plus 1fill+1pt plus 1fil\relax}
\V{\the\glueexpr 1pt plus 1filll minus 1fil+2pt plus 1fil minus 1filll\relax}
\V{\the\glueexpr (1pt plus 2pt)*3\relax}\V{\the\glueexpr (1pt plus 2pt minus 1pt)/3\relax}\V{\the\glueexpr -(1pt plus 2fil)\relax}
\V{\the\glueexpr 1pt plus 3fil*2/3\relax}
\V{\the\glueexpr \skip0\relax}\V{\the\glueexpr\baselineskip*2\relax}\V{\the\glueexpr\parindent\relax}\V{\the\glueexpr 1\relax}
\V{\the\glueexpr 16383pt plus 16383pt+16383pt plus 16383pt\relax}
\V{\the\glueexpr 16383pt plus 16383pt*2\relax}
\V{\the\glueexpr 1pt plus 1pt/0\relax}
\V{\the\glueexpr\relax}\V{\the\glueexpr (1pt\relax}\V{\the\glueexpr 1pt plus\relax}\V{\the\glueexpr x\relax}\V{\the\glueexpr\thinmuskip\relax}
\skip1=\glueexpr 1pt plus 1fil*2\relax\showthe\skip1
\setbox1=\hbox to 10pt{\hskip\glueexpr 0pt plus 1fil\relax}\showbox1
\showthe\glueexpr 1pt\relax
\glueexpr 1pt\relax
\show\glueexpr
\dimen1=\glueexpr 1pt plus 1pt\relax\showthe\dimen1
\count1=\glueexpr 1pt plus 1pt\relax\showthe\count1
""")
P('muexpr', r"""
\T{muexpr}
\V{\the\muexpr 1mu plus 2mu minus 3mu\relax}\V{\the\muexpr 1mu plus 2fil minus 3fill\relax}
\V{\the\muexpr 1mu plus 1fil+1mu plus 1fill\relax}\V{\the\muexpr (1mu plus 2mu)*3\relax}\V{\the\muexpr (1mu plus 2mu)/3\relax}
\V{\the\muexpr -(1mu)\relax}\V{\the\muexpr\muskip0\relax}\V{\the\muexpr\thinmuskip*2\relax}
\V{\the\muexpr 1pt\relax}\V{\the\muexpr\skip0\relax}\V{\the\muexpr 1\relax}
\V{\the\muexpr\relax}\V{\the\muexpr (1mu\relax}\V{\the\muexpr x\relax}\V{\the\muexpr 16383mu*2\relax}\V{\the\muexpr 1mu/0\relax}
\muskip1=\muexpr 1mu*2\relax\showthe\muskip1
\setbox1=\hbox{$a\mskip\muexpr 1mu\relax b$}\showbox1
\showthe\muexpr 1mu\relax
\muexpr 1mu\relax
\show\muexpr
""")
P('gluestretch glueshrink gluestretchorder glueshrinkorder', r"""
\T{glue parts}
\def\g#1{\V{\the\gluestretch\glueexpr#1\relax,\the\glueshrink\glueexpr#1\relax,\the\gluestretchorder\glueexpr#1\relax,\the\glueshrinkorder\glueexpr#1\relax}}
\g{1pt}\g{1pt plus 2pt}\g{1pt minus 2pt}\g{1pt plus 2pt minus 3pt}\g{1pt plus 2fil minus 3fill}\g{1pt plus 2filll minus 3fil}
\skip1=1pt plus 2fil minus 3fill \V{\the\gluestretch\skip1 \the\glueshrink\skip1 \the\gluestretchorder\skip1 \the\glueshrinkorder\skip1}
\V{\the\gluestretch\baselineskip}\V{\the\gluestretchorder\parskip}
\V{\the\gluestretch 1pt}\V{\the\glueshrink x}\V{\the\gluestretchorder\relax}\V{\the\glueshrinkorder 5}
\V{\the\gluestretch\thinmuskip}
\gluestretch\skip1
\glueshrink\skip1
\gluestretchorder\skip1
\glueshrinkorder\skip1
\gluestretch\skip1=1pt
\showthe\gluestretch\skip1 \showthe\glueshrink\skip1 \showthe\gluestretchorder\skip1 \showthe\glueshrinkorder\skip1
""")
P('mutoglue gluetomu', r"""
\T{mu and glue conversion}
\muskip1=3mu plus 2mu minus 1fil \V{\the\mutoglue\muskip1}\skip1=\mutoglue\muskip1 \showthe\skip1
\skip1=3pt plus 2pt minus 1fil \V{\the\gluetomu\skip1}\muskip1=\gluetomu\skip1 \showthe\muskip1
\V{\the\mutoglue\thinmuskip}\V{\the\gluetomu\baselineskip}\V{\the\mutoglue\muexpr 1mu\relax}\V{\the\gluetomu\glueexpr 1pt\relax}
\V{\the\mutoglue x}\V{\the\gluetomu x}\V{\the\mutoglue 1pt}\V{\the\gluetomu 1mu}\V{\the\mutoglue\skip1}\V{\the\gluetomu\muskip1}
\mutoglue\muskip1
\gluetomu\skip1
\setbox1=\hbox{\hskip\mutoglue\muskip1}\showbox1
\setbox1=\hbox{$\mskip\gluetomu\skip1$}\showbox1
""")
# ---------------------------------------------------------------------------------------------- e-TeX introspection
P('currentgrouplevel currentgrouptype', r"""
\T{group introspection}
\def\g{\V{\the\currentgrouplevel,\the\currentgrouptype}}
\g
{\g{\g}}
\begingroup\g\endgroup
\setbox1=\hbox{\g}
\setbox1=\vbox{\g}
\setbox1=\vtop{\g}
\setbox1=\hbox{$\g\left(\g\right)\vcenter{\g}\mathchoice{\g}{}{}{}a^{\g}$}
\setbox1=\hbox{\discretionary{\g}{\g}{\g}}
\setbox1=\vbox{\halign{#\g\cr\noalign{\g}a\cr}}
\setbox1=\hbox{\valign{#\g\cr\hbox{a}\cr}}
\insert100{\g}\noindent\vadjust{\g}\par
\noindent$\g$\par
$$\g$$
\output={\g\setbox0=\box255 \deadcycles=0 }\hbox{}\penalty-10000
\setbox1=\hbox{\moveleft1pt\hbox{\g}\raise1pt\vbox{\g}}
\setbox1=\hbox{\accent127 \g x}
\showthe\currentgrouplevel\showthe\currentgrouptype
\currentgrouplevel=1
\currentgrouptype=1
\currentgrouplevel
\currentgrouptype
""")
P('currentiflevel currentiftype currentifbranch', r"""
\T{conditional introspection}
\def\i{\V{\the\currentiflevel,\the\currentiftype,\the\currentifbranch}}
\i
\iftrue\i\else\fi
\iffalse\else\i\fi
\unless\iftrue\else\i\fi
\unless\iffalse\i\fi
\ifcase0 \i\or\fi\ifcase1 \or\i\fi\ifcase2 \or\else\i\fi
\if aa\i\fi\ifcat aa\i\fi\ifx aa\i\fi\ifnum1=1 \i\fi\ifdim1pt=1pt \i\fi\ifodd1 \i\fi
\ifvmode\i\fi\unless\ifhmode\i\fi\unless\ifmmode\i\fi\unless\ifinner\i\fi\ifvoid0 \i\fi\unless\ifhbox0 \i\fi\unless\ifvbox0 \i\fi\ifeof0 \i\fi
\ifdefined\i\i\fi\ifcsname i\endcsname\i\fi\iffontchar\tenrm65 \i\fi\unless\ifincsname\i\fi\ifpdfprimitive\relax\i\fi\ifpdfabsnum1=1 \i\fi\ifpdfabsdim1pt=1pt \i\fi
\iftrue\iftrue\iftrue\i\fi\fi\fi
\showthe\currentiflevel\showthe\currentiftype\showthe\currentifbranch
\currentiflevel=1
\currentiftype
\currentifbranch
""")
P('showgroups', r"""
\T{showgroups}
\showgroups
{\showgroups{\showgroups}}
\begingroup\showgroups\endgroup
\setbox1=\hbox{\showgroups}
\setbox1=\vbox{\showgroups}
\setbox1=\vtop{\showgroups}
\setbox1=\hbox to 10pt{\showgroups}
\setbox1=\hbox spread 10pt{\showgroups}
\setbox1=\hbox{$\showgroups\left(\showgroups\right)\vcenter{\showgroups}\mathchoice{\showgroups}{\showgroups}{\showgroups}{\showgroups}a^{\showgroups}$}
\setbox1=\hbox{\discretionary{\showgroups}{\showgroups}{\showgroups}}
\setbox1=\vbox{\halign{#\showgroups\cr\noalign{\showgroups}a\cr}}
\setbox1=\hbox{\valign{#\showgroups\cr\hbox{a}\cr}}
\insert100{\showgroups}\noindent\vadjust{\showgroups}\par
\noindent\vadjust pre{\showgroups}\par
\noindent$\showgroups$\par
$$\showgroups a\eqno\showgroups b$$
\output={\showgroups\setbox0=\box255 \deadcycles=0 }\hbox{}\penalty-10000
\setbox1=\hbox{\moveleft1pt\hbox{\showgroups}\raise1pt\vbox{\showgroups}\lower 1pt\vtop{\showgroups}}
\setbox1=\vbox{\moveright 1pt\vbox{\showgroups}}
\setbox1=\hbox{\leaders\hbox{\showgroups}\hfil}
\shipout\hbox{\showgroups}
\global\showgroups
\setbox1=\hbox{\accent127 \begingroup\showgroups\endgroup x}
""")
P('showifs', r"""
\T{showifs}
\showifs
\iftrue\showifs\fi
\iffalse\else\showifs\fi
\unless\iffalse\showifs\fi
\ifcase0 \showifs\or\fi\ifcase1 \or\showifs\else\fi\ifcase5 \or\else\showifs\fi
\iftrue\iftrue\iffalse\else\if aa\showifs\fi\fi\fi\fi
\if aa\showifs\fi\ifcat aa\showifs\fi\ifx aa\showifs\fi\ifnum1=1 \showifs\fi\ifdim1pt=1pt \showifs\fi
\ifodd1 \showifs\fi\ifvmode\showifs\fi\ifvoid0 \showifs\fi\ifeof0 \showifs\fi
\ifdefined\relax\showifs\fi\ifcsname relax\endcsname\showifs\fi\iffontchar\tenrm65 \showifs\fi
\ifpdfprimitive\relax\showifs\fi\ifpdfabsnum1=1 \showifs\fi\ifpdfabsdim1pt=1pt \showifs\fi
\errorcontextlines=-1 \iftrue\showifs\fi\errorcontextlines=4
\def\s{\showifs}\iftrue\s\fi
\setbox1=\hbox{\iftrue\showifs}\fi
""")
P('showtokens', r"""
\T{showtokens}
\showtokens{a b\c#}
\showtokens{}
\showtokens\expandafter{\jobname}
\def\m{\relax x}\showtokens\expandafter{\m}
\toks0={t}\showtokens\expandafter{\the\toks0}
\showtokens\toks0
\showtokens x
\showtokens\relax{y}
\showtokens{\iffalse}
\showtokens{}}
\setbox1=\hbox{\showtokens{in box}}
\showtokens{xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx}
\errorcontextlines=-1 \showtokens{no context}\errorcontextlines=4
\def\deep{\showtokens{inside macro}}\deep
""")
P('detokenize', r"""
\T{detokenize}
\V{\detokenize{a b\c #{d}}}\V{\detokenize{}}\V{\detokenize{\a\b \c}}\V{\detokenize{ a }}\V{\detokenize{##}}
\V{\detokenize\expandafter{\jobname}}\def\m{\relax x}\V{\detokenize\expandafter{\m}}
\edef\a{\detokenize{a\b}}\show\a
\edef\a{\detokenize{ }}\show\a
\catcode`\~=13 \V{\detokenize{~}}\catcode`\~=12 \V{\detokenize{~}}
\escapechar=-1 \V{\detokenize{\a}}\escapechar=`! \V{\detokenize{\a}}\escapechar=`\\
\V{\detokenize{^^80^^ff}}
\V{\the\numexpr\detokenize{5}\relax}
\expandafter\show\detokenize{a}
\V{\detokenize x}\par
\V{\detokenize\relax{y}}
\detokenize{in vertical}\par
\setbox1=\hbox{\detokenize{a b}}\showbox1
\V{\detokenize{\iffalse}}\V{\detokenize{\fi}}
\toks0={a#b}\V{\detokenize\expandafter{\the\toks0}}
\V{\detokenize\toks0}
\show\detokenize
\V{\detokenize{xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx}}
""")
P('unexpanded', r"""
\T{unexpanded}
\def\b{B}\edef\a{\unexpanded{\b}\b}\show\a
\edef\a{\unexpanded{#}}\show\a
\edef\a{\unexpanded{##}}\show\a
\edef\a{\unexpanded\expandafter{\b}}\show\a
\edef\a{\unexpanded{}}\show\a
\xdef\a{\unexpanded{\b}}\show\a
\V{\unexpanded{\b}}\message{\unexpanded{\b}}
\toks0={x}\toks1=\expandafter{\unexpanded{\b}}\showthe\toks1
\edef\a{\the\toks0\unexpanded{\the\toks0}}\show\a
\unexpanded{\relax}
\setbox1=\hbox{\unexpanded{b}}\showbox1
\expandafter\show\unexpanded{\b}
\edef\a{\unexpanded x}
\edef\a{\unexpanded\b{y}}\show\a
\edef\a{\unexpanded\toks0}\show\a
\def\a{\unexpanded{\b}}\edef\c{\a}\show\c
\edef\a{\unexpanded{\iffalse}}\show\a
\edef\a{\unexpanded{\fi}}\show\a
\unexpanded{}}
\show\unexpanded
\immediate\write-1{\unexpanded{\b#}}
\mark{\unexpanded{\b}}\special{\unexpanded{\b}}
""")
P('protected', r"""
\T{protected}
\protected\def\p{P}\show\p\V{\meaning\p}\edef\a{\p}\show\a
\protected\long\def\p#1{#1}\show\p
\long\protected\outer\def\o{o}\show\o
\protected\edef\p{x}\show\p
\protected\gdef\p{y}\show\p
\protected\xdef\p{z}\show\p
\protected\relax
\protected x\par
\protected\let\a=\relax
\protected\global\def\p{g}\show\p
\global\protected\def\p{h}\show\p
\protected\def\p{P}\immediate\write-1{\p}\message{\p}
\mark{\p}\showtokens\expandafter{\p}
\setbox1=\hbox{\p}\showbox1
\halign{\p#\cr a\cr}
\def\t{\p}\edef\u{\t}\show\u
\expandafter\show\p
\ifx\p\undefinedthing\V{no}\else\V{yes}\fi
\count1=1\p\par
\if\p\relax\V{a}\else\V{b}\fi
\edef\a{\the\numexpr1\p}\show\a
\csname\p\endcsname
\show\protected
\def\z{\protected}\z\def\y{1}\show\y
""")
P('unless', r"""
\T{unless}
\unless\iftrue\V{a}\else\V{b}\fi\unless\iffalse\V{c}\else\V{d}\fi
\unless\ifnum1=1 \V{e}\else\V{f}\fi\unless\ifdim1pt=2pt \V{g}\else\V{h}\fi
\unless\ifx aa\V{i}\else\V{j}\fi\unless\if aa\V{k}\else\V{l}\fi\unless\ifcat aa\V{m}\else\V{n}\fi
\unless\ifodd1 \V{o}\else\V{p}\fi\unless\ifvmode\V{q}\else\V{r}\fi\unless\ifhmode\V{s}\else\V{t}\fi\unless\ifmmode\V{u}\else\V{v}\fi\unless\ifinner\V{w}\else\V{x}\fi
\unless\ifvoid0 \V{a}\else\V{b}\fi\unless\ifhbox0 \V{c}\else\V{d}\fi\unless\ifvbox0 \V{e}\else\V{f}\fi\unless\ifeof0 \V{g}\else\V{h}\fi
\unless\ifdefined\relax\V{a}\else\V{b}\fi\unless\ifcsname relax\endcsname\V{c}\else\V{d}\fi\unless\iffontchar\tenrm65 \V{e}\else\V{f}\fi
\unless\ifpdfprimitive\relax\V{a}\else\V{b}\fi\unless\ifpdfabsnum1=1 \V{c}\else\V{d}\fi\unless\ifpdfabsdim1pt=1pt \V{e}\else\V{f}\fi\unless\ifincsname\V{g}\else\V{h}\fi
\T{unless errors}
\unless\ifcase0 \fi
\unless\relax
\unless x
\unless\fi
\unless\else
\unless\unless\iftrue\fi
\unless
\relax
\show\unless
\def\u{\unless\iffalse}\u\V{via macro}\fi
\expandafter\unless\csname iftrue\endcsname\V{x}\else\V{y}\fi
\let\t=\iftrue\unless\t\V{a}\else\V{b}\fi
\unless\t\V{c}\unless\t\V{d}\else\V{e}\fi\else\V{f}\fi
""")
P('ifdefined', r"""
\T{ifdefined}
\ifdefined\relax\V{a}\else\V{b}\fi\ifdefined\undefinedthing\V{c}\else\V{d}\fi
\def\m{}\ifdefined\m\V{e}\else\V{f}\fi\let\n=\undefinedthing\ifdefined\n\V{g}\else\V{h}\fi
\ifdefined a\V{i}\else\V{j}\fi\ifdefined{\V{k}\else\V{l}\fi}
\ifdefined~\V{m}\else\V{n}\fi
\catcode`\~=13 \let~=\undefinedthing\ifdefined~\V{o}\else\V{p}\fi\catcode`\~=12
\expandafter\ifdefined\csname q\endcsname\V{r}\else\V{s}\fi
\outer\def\o{}\ifdefined\o\V{t}\else\V{u}\fi
\ifdefined\fi
\ifdefined\else\V{after else}\fi
\show\ifdefined\showifs
""")
P('ifcsname', r"""
\T{ifcsname}
\ifcsname relax\endcsname\V{a}\else\V{b}\fi\ifcsname undefinedthing\endcsname\V{c}\else\V{d}\fi
\ifcsname\endcsname\V{e}\else\V{f}\fi\ifcsname a b\endcsname\V{g}\else\V{h}\fi
\def\n{relax}\ifcsname\n\endcsname\V{i}\else\V{j}\fi\ifcsname\string\relax\endcsname\V{k}\else\V{l}\fi
\ifcsname\relax\endcsname\V{m}\else\V{n}\fi
\expandafter\def\csname ok\endcsname{}\ifcsname ok\endcsname\V{o}\else\V{p}\fi
\expandafter\let\csname und\endcsname=\undefinedthing\ifcsname und\endcsname\V{q}\else\V{r}\fi
\ifcsname noneyet\endcsname\fi\expandafter\show\csname noneyet\endcsname
\ifcsname\par\endcsname\V{s}\else\V{t}\fi
\ifcsname\fi
\show\ifcsname
\ifcsname^^80\endcsname\V{u}\else\V{v}\fi
\ifcsname\ifincsname yes\else no\fi\endcsname\V{w}\else\V{x}\fi
""")
P('iffontchar', r"""
\T{iffontchar}
\iffontchar\tenrm65 \V{a}\else\V{b}\fi\iffontchar\tenrm200 \V{c}\else\V{d}\fi\iffontchar\tenrm255 \V{e}\else\V{f}\fi
\iffontchar\nullfont65 \V{g}\else\V{h}\fi
\iffontchar\font65 \V{i}\else\V{j}\fi\iffontchar\textfont0 65 \V{k}\else\V{l}\fi
\iffontchar\tenrm256 \V{m}\else\V{n}\fi
\iffontchar\tenrm-1 \V{o}\else\V{p}\fi
\iffontchar x65 \V{q}\else\V{r}\fi
\iffontchar\relax65 \V{s}\else\V{t}\fi
\iffontchar\tenrm x\V{u}\else\V{v}\fi
\iffontchar\fi
\show\iffontchar
""")
P('fontcharwd fontcharht fontchardp fontcharic', r"""
\T{char metrics}
\def\c#1#2{\V{\the\fontcharwd#1#2,\the\fontcharht#1#2,\the\fontchardp#1#2,\the\fontcharic#1#2}}
\c\tenrm{65}\c\tenrm{103}\c\tenit{102}\c\tenrm{200}\c\tenrm{255}\c\nullfont{65}\c\font{65}\c\tensy{0}\c\tenex{0}
\V{\the\fontcharwd\tenrm256}\V{\the\fontcharht\tenrm-1}\V{\the\fontchardp x65}\V{\the\fontcharic\relax65}\V{\the\fontcharwd\tenrm x}
\fontcharwd\tenrm65
\fontcharht\tenrm65
\fontchardp\tenrm65
\fontcharic\tenrm65
\fontcharwd\tenrm65=1pt
\showthe\fontcharwd\tenrm65 \showthe\fontcharht\tenrm65 \showthe\fontchardp\tenrm65 \showthe\fontcharic\tenit102
\font\f=cmr10 at 20pt \V{\the\fontcharwd\f65}\dimen1=\fontcharwd\f65 \showthe\dimen1
""")
P('parshapelength parshapeindent parshapedimen', r"""
\T{parshape introspection}
\def\s{\V{\the\parshapelength0,\the\parshapelength1,\the\parshapelength2,\the\parshapelength3,\the\parshapelength4,\the\parshapelength-1
  ;\the\parshapeindent0,\the\parshapeindent1,\the\parshapeindent2,\the\parshapeindent3,\the\parshapeindent4,\the\parshapeindent-1
  ;\the\parshapedimen0,\the\parshapedimen1,\the\parshapedimen2,\the\parshapedimen3,\the\parshapedimen4,\the\parshapedimen6,\the\parshapedimen7,\the\parshapedimen-1}}
\s
\parshape=3 1pt 11pt 2pt 12pt 3pt 13pt \s
\parshape=1 1pt 11pt \s
\parshape=0 \s
\V{\the\parshapelength x}\V{\the\parshapeindent x}\V{\the\parshapedimen x}
\parshapelength1
\parshapeindent1
\parshapedimen1
\parshapelength1=1pt
\showthe\parshapelength1 \showthe\parshapeindent1 \showthe\parshapedimen1
""")
P('lastnodetype', r"""
\T{lastnodetype}
\showthe\lastnodetype
\def\l{\V{\the\lastnodetype}}
\setbox1=\hbox{\l a\l\hbox{}\l\vbox{}\l\vrule\l\hskip1pt\l\kern1pt\l\penalty0 \l\mark{m}\l\insert100{}\l\discretionary{}{}{}\l\special{s}\l\vadjust{}\l$a$\l}
\setbox1=\vbox{\l\hbox{}\l\vbox{}\l\hrule\l\vskip1pt\l\kern1pt\l\penalty0 \l\mark{m}\l\insert100{}\l\special{s}\l}
\setbox1=\hbox{$\l a\l\hskip1pt\l\mkern1mu\l\mskip1mu\l\mathchoice{}{}{}{}\l\left(\l\right)\l\mathsurround=1pt \l\penalty0 \l$\l}
\setbox1=\hbox{a\accent127 e\l\/\l fi\l\TeXXeTstate=1 \beginL\l\endL\l\TeXXeTstate=0 }
\noindent\l a\l\par\l
\hbox{}\l\kern1pt\l\vskip1pt\l\penalty0 \l
\lastnodetype=1
\lastnodetype
""")
P('interactionmode', r"""
\T{interactionmode}
\showthe\interactionmode
\interactionmode=0 \showthe\interactionmode
\interactionmode=1 \showthe\interactionmode
\interactionmode=2 \showthe\interactionmode
\interactionmode=0 \showthe\interactionmode
\interactionmode=4
\interactionmode=-1
\interactionmode=2147483647
\interactionmode=x
\begingroup\interactionmode=1 \endgroup\showthe\interactionmode
\begingroup\global\interactionmode=1 \endgroup\showthe\interactionmode
\advance\interactionmode by 1 \showthe\interactionmode
\multiply\interactionmode by 0 \showthe\interactionmode
\undefinedA
\nonstopmode\showthe\interactionmode\scrollmode\showthe\interactionmode\batchmode\showthe\interactionmode
\show\interactionmode
""")
P('eTeXversion eTeXrevision', r"""
\T{eTeX version}
\V{\the\eTeXversion}\V{\eTeXrevision}\showthe\eTeXversion
\eTeXversion=3
\eTeXversion
\eTeXrevision\par
\edef\a{\eTeXrevision}\show\a
\number\eTeXversion\par
""")
# ---------------------------------------------------------------------------------------------- e-TeX input
P('scantokens', r"""
\T{scantokens}
\scantokens{\V{inside}}\V{after}
\edef\a{\scantokens{\noexpand\relax}}
\def\s{}\expandafter\def\expandafter\s\expandafter{\scantokens{abc\noexpand}}\show\s
\everyeof={\noexpand}\edef\a{\scantokens{abc}}\show\a\everyeof={}
\catcode`\Q=12 \scantokens{\V{Q is other}}\catcode`\Q=11
\def\v{\catcode`\%=12 \scantokens}\begingroup\v{\V{a%b}}\endgroup
\scantokens{a\par}
\scantokens{\V{line one}
\V{line two \the\inputlineno}
}
\V{\the\inputlineno}
\scantokens{}
\scantokens{\endinput \V{not here}}
\scantokens\expandafter{\jobname\par}
\toks0={\V{from toks}}\scantokens\expandafter{\the\toks0}
\begingroup\tracingscantokens=1 \scantokens{\V{traced}}\endgroup
\setbox1=\hbox{\scantokens{x}}\showbox1
\scantokens{\scantokens{\scantokens{\V{nested}}}}
\scantokens x
\scantokens\relax{y}
\show\scantokens
\newlinechar=`| \scantokens{\V{one}|\V{two}}\newlinechar=-1
\endlinechar=-1 \scantokens{\V{no endlinechar}}\endlinechar=13
\catcode`\^^M=12 \scantokens{\V{cr other}}\catcode`\^^M=5
\scantokens{\errmessage{in scantokens}}
\scantokens{\def\p#1{}\p}\relax
\scantokens{\iffalse}\fi
\scantokens{\begingroup}\endgroup
\scantokens{\undefinedthing}
\everyeof={\relax}\V{\scantokens{x}}\everyeof={}
\edef\a{\scantokens{y}}
\show\a
""")
P('everyeof', r"""
\T{everyeof}
\showthe\everyeof
\immediate\openout1=probeeof.tex \immediate\write1{\string\V{in file}}\immediate\closeout1
\everyeof={\V{eof}}\input probeeof.tex \scantokens{\V{st}}
\everyeof={\relax}\showthe\everyeof
\edef\a{\scantokens{x}}\show\a
\everyeof={\noexpand}\edef\a{\scantokens{x}}\show\a
\everyeof={\the\everyeof}\input probeeof.tex
\everyeof={}
\begingroup\everyeof={\V{local}}\scantokens{}\endgroup\scantokens{}
\everyeof=x
\everyeof=\toks0
\everyeof\toks0
\V{\the\everyeof}
\everyeof={\input probeeof}
\everyeof={}
\show\everyeof
""")
# ---------------------------------------------------------------------------------------------- marks classes
P('marks', r"""
\T{marks}
\marks0{zero}\marks1{one}\marks255{big}\marks256{bigger}\marks32767{biggest}
\marks32768{toobig}
\marks-1{neg}
\marks x{y}
\marks3 x\par
\setbox1=\vbox{\marks3{in box}}\showbox1
\setbox1=\hbox{\marks3{in hbox}\marks4{b}}\showbox1
\setbox1=\hbox{$\marks3{in math}$}\showbox1
\def\m{M}\setbox1=\vbox{\marks3{\m\noexpand\m}}\showbox1
\noindent a\marks3{in par}b\par
\marks3{}}
\show\marks
\def\all{\V{[\topmarks3][\firstmarks3][\botmarks3][\topmarks0][\firstmarks0][\botmarks0][\topmarks4][\firstmarks4][\botmarks4]}}
\all\vfill\penalty-10000 \all
\hbox{}\marks3{a}\marks4{b}\marks0{c}\vfill\penalty-10000 \all
\hbox{}\marks3{d}\vfill\penalty-10000 \all
\hbox{}\vfill\penalty-10000 \all
\marks3{unused}\marks5{other}\vfill\penalty-10000 \all
""")
P('topmarks firstmarks botmarks splitfirstmarks splitbotmarks', r"""
\T{marks classes}
\def\p{\V{[\topmarks3][\firstmarks3][\botmarks3][\splitfirstmarks3][\splitbotmarks3]}}
\p\hbox{}\marks3{a}\marks3{b}\vfill\penalty-10000 \p
\hbox{}\marks3{c}\vfill\penalty-10000 \p
\hbox{}\vfill\penalty-10000 \p
\setbox1=\vbox{\hbox{}\marks3{s1}\penalty0 \hbox{}\marks3{s2}\penalty0 \hbox{}\marks3{s3}\marks9{n}}
\setbox2=\vsplit1 to 1pt \p\V{[\splitfirstmarks9][\splitbotmarks9]}
\setbox2=\vsplit1 to 1pt \p\V{[\splitfirstmarks9][\splitbotmarks9]}
\setbox2=\vsplit1 to 1000pt \p\V{[\splitfirstmarks9][\splitbotmarks9]}
\V{\topmarks32767 \firstmarks32767 \botmarks32767 \splitfirstmarks32767 \splitbotmarks32767}
\V{\topmarks32768}\V{\firstmarks-1}\V{\botmarks x}\V{\splitfirstmarks32768}\V{\splitbotmarks-1}
\edef\a{\topmarks3\firstmarks3\botmarks3\splitfirstmarks3\splitbotmarks3}\show\a
\topmarks3\par\firstmarks3\par\botmarks3\par\splitfirstmarks3\par\splitbotmarks3\par
\show\topmarks\show\firstmarks\show\botmarks\show\splitfirstmarks\show\splitbotmarks
""")
# ---------------------------------------------------------------------------------------------- paragraph penalties
P('interlinepenalties clubpenalties widowpenalties displaywidowpenalties', r"""
\T{penalty arrays}
\def\arr#1{\showthe#10 \showthe#11 \showthe#12 \showthe#13 \showthe#14 \showthe#1-1
  #1=3 100 200 300 \showthe#1\showthe#10 \showthe#11 \showthe#12 \showthe#13 \showthe#14 \showthe#1-1
  \begingroup#1=1 7 \showthe#11 \showthe#12 \endgroup\showthe#11
  #1=0 \showthe#1\showthe#10 \showthe#11
  #1=-1 \showthe#1\showthe#10
  #1=1 x
  \par#1=x
  \par#1=2 1
  \par#1=0
  #1=1 2147483647 \showthe#11 #1=1 2147483648 \par#1=0 }
\arr\interlinepenalties
\arr\clubpenalties
\arr\widowpenalties
\arr\displaywidowpenalties
\def\t{one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty}
\hsize=50pt
\interlinepenalties=3 101 102 103 \clubpenalties=2 201 202 \widowpenalties=2 301 302
\setbox1=\vbox{\noindent\t\par}\showbox1
\displaywidowpenalties=2 401 402
\setbox1=\vbox{\noindent\t$$a$$\t$$b$$\par}\showbox1
\interlinepenalties=2 11 12 \setbox1=\vbox{\noindent\t\par}\showbox1
\interlinepenalties=2147483647
\interlinepenalties=0 \clubpenalties=0 \widowpenalties=0 \displaywidowpenalties=0 \hsize=200pt
\advance\interlinepenalties by 1
\interlinepenalties=100000000
\interlinepenalties=0
""")
P('lastlinefit', r"""
\T{lastlinefit}
\showthe\lastlinefit
\def\t{one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen}
\hsize=100pt \parfillskip=0pt plus 1fil
\def\r#1{\lastlinefit=#1 \setbox1=\vbox{\tracingparagraphs=1 \noindent\t\par}\showbox1 }
\r{0}\r{1000}\r{500}\r{1}\r{-1}\r{1001}
\parfillskip=0pt plus 1pt \r{1000}
\parfillskip=0pt minus 5pt \r{1000}
\parfillskip=10pt plus 1fil minus 1pt \r{1000}
\parfillskip=0pt plus 1fill \r{1000}
\setbox1=\vbox{\lastlinefit=1000 \parfillskip=0pt plus 1fil \noindent\t\par\showthe\prevgraf}\showbox1
\setbox1=\vbox{\lastlinefit=1000 \parfillskip=0pt plus 1fil \noindent\hbox to 500pt{}\par}\showbox1
\setbox1=\vbox{\lastlinefit=1000 \parfillskip=0pt plus 1fil \noindent a\hskip 0pt plus 1fil b\par}\showbox1
\setbox1=\vbox{\lastlinefit=1000 \parfillskip=0pt plus 1fil \looseness=1 \noindent\t\par}\showbox1
\setbox1=\vbox{\lastlinefit=1000 \parfillskip=0pt plus 1fil \emergencystretch=50pt \tolerance=100 \noindent\t\par}\showbox1
\lastlinefit=0 \hsize=200pt \parfillskip=0pt plus 1fil
""")
# ---------------------------------------------------------------------------------------------- vertical discards
P('pagediscards splitdiscards savingvdiscards', r"""
\T{discards}
\savingvdiscards=1 \showthe\savingvdiscards
\setbox1=\vbox{\hbox{a}\kern1pt\penalty0 \vskip2pt\kern3pt\hbox{b}\kern4pt}
\setbox2=\vsplit1 to 6pt \showbox1\showbox2
\setbox3=\vbox{\splitdiscards}\showbox3
\setbox3=\vbox{\splitdiscards}\showbox3
\setbox1=\vbox{\hbox{a}\penalty0 \vskip5pt}\setbox2=\vsplit1 to 100pt \setbox3=\vbox{\splitdiscards}\showbox3
\output={\setbox3=\vbox{\pagediscards}\showbox3 \setbox3=\vbox{\pagediscards}\showbox3 \shipout\box255 }
\hbox{a}\penalty-10000 \vskip1pt\penalty5 \kern2pt\hbox{b}\vfill\penalty-10000
\savingvdiscards=0
\setbox1=\vbox{\hbox{a}\penalty0 \vskip5pt\hbox{b}}\setbox2=\vsplit1 to 6pt \setbox3=\vbox{\splitdiscards}\showbox3
\pagediscards
\splitdiscards
\setbox3=\hbox{\pagediscards}\showbox3
\setbox3=\hbox{\splitdiscards}\showbox3
\setbox3=\hbox{$\pagediscards$}\showbox3
\setbox3=\hbox{$\splitdiscards$}\showbox3
\noindent\splitdiscards\pagediscards\par
\savingvdiscards=2147483647 \showthe\savingvdiscards \savingvdiscards=-1 \showthe\savingvdiscards \savingvdiscards=0
\show\pagediscards \show\splitdiscards
\output={\global\setbox7=\box255 \deadcycles=0 }
\hbox{}\penalty-10000
""")
P('savinghyphcodes', r"""
\T{savinghyphcodes}
\showthe\savinghyphcodes
\savinghyphcodes=1 \showthe\savinghyphcodes
\patterns{1a}
\savinghyphcodes=-1 \showthe\savinghyphcodes
\savinghyphcodes=2147483647 \savinghyphcodes=0
\setbox1=\vbox{\hsize=10pt\savinghyphcodes=1 \noindent typesetting\par}\showbox1
""")
# ---------------------------------------------------------------------------------------------- TeXXeT
P('TeXXeTstate beginL beginR endL endR predisplaydirection', r"""
\T{TeXXeT}
\showthe\TeXXeTstate
\setbox1=\hbox{\beginL a\endL}\showbox1
\beginR
\endR
\TeXXeTstate=1 \showthe\TeXXeTstate
\def\s#1{\setbox1=\hbox{#1}\showbox1 \shipout\copy1 }
\s{\beginL a\endL}\s{\beginR abc\endR}\s{\beginR a\beginL b\endL c\endR}\s{\beginL a\beginR b\endR c\endL}
\s{\beginR a}\s{a\endR}\s{a\endL}\s{\beginR a\endL}\s{\beginL a\endR}\s{\beginL\beginR\endL\endR}
\s{\beginR a\kern1pt\vrule width 2pt\hskip3pt\penalty0 \hbox{b}\vbox{}\discretionary{x}{y}{z}\endR}
\s{\beginR\hskip 3pt plus 1fil\leaders\hrule\hskip 3pt\endR}
\s{\beginR\accent127 e fi\/\endR}
\s{\beginR\pdfsave\pdfrestore\special{s}\write-1{w}\endR}
\s{\beginR$a+b$\endR}\s{$\beginR a\endR$}\s{$\beginL$}
\setbox1=\hbox to 100pt{\beginR a\hfil b\endR}\showbox1 \shipout\copy1
\setbox1=\vbox{\hsize=50pt \noindent\beginR one two three four five six seven eight\endR\par}\showbox1\shipout\copy1
\setbox1=\vbox{\hsize=50pt \noindent\beginR one two three four five six seven eight\par}\showbox1\shipout\copy1
\setbox1=\vbox{\hsize=50pt \noindent\beginR one two\endR three \beginL four\endL five six seven eight\par}\showbox1\shipout\copy1
\setbox1=\vbox{\noindent$$\predisplaydirection=1 \showthe\predisplaydirection a\eqno(1)$$\par}\showbox1\shipout\copy1
\setbox1=\vbox{\noindent$$\predisplaydirection=-1 a\eqno(1)$$\par}\showbox1\shipout\copy1
\setbox1=\vbox{\noindent\beginR b$$\showthe\predisplaydirection a\leqno(1)$$\par}\showbox1\shipout\copy1
\setbox1=\vbox{\noindent\beginR b$$\showthe\predisplaysize\showthe\displaywidth a$$c\endR\par}\showbox1\shipout\copy1
\setbox1=\hbox{\beginR a\endR}\setbox2=\hbox{\unhbox1 \unskip\unkern\unpenalty}\showbox2
\setbox1=\hbox{\beginR a}\setbox2=\hbox{\beginL\unhbox1 }\showbox2
\beginL
\endL
\noindent\beginL a\endL\par
\predisplaydirection=1 \showthe\predisplaydirection\predisplaydirection=0
\TeXXeTstate=-1 \showthe\TeXXeTstate \TeXXeTstate=2147483647 \showthe\TeXXeTstate
\TeXXeTstate=0 \showthe\TeXXeTstate
\s{\beginL}
\begingroup\TeXXeTstate=1 \s{\beginR a\endR}\endgroup\s{\endR}
\show\beginL\show\beginR\show\endL\show\endR
""")
# ---------------------------------------------------------------------------------------------- tracing
P('tracingassigns', r"""
\T{tracingassigns}
\tracingassigns=1
\count1=1 \count1=1 \count1=2 \dimen1=1pt \dimen1=1pt \skip1=1pt \muskip1=1mu \toks1={a}\toks1={a}\def\a{b}\def\a{b}\def\a{c}
\catcode`\Q=12 \catcode`\Q=12 \catcode`\Q=11 \tolerance=100 \tolerance=100 \hsize=1pt \hsize=1pt \baselineskip=1pt \thinmuskip=1mu \hsize=200pt
\let\a=\relax\let\a=\relax\chardef\a=5 \chardef\a=5 \font\f=cmr10 \font\f=cmr10 \tenrm\tenrm
\textfont0=\tenrm\textfont0=\tenrm\setbox1=\box2 \setbox1=\hbox{}\everypar={x}\everypar={x}\everypar={}\everypar={}
\parshape=1 1pt 1pt \parshape=1 1pt 1pt \parshape=0 \parshape=0 \interlinepenalties=1 1 \interlinepenalties=1 1 \interlinepenalties=0
\global\count1=3 \global\def\a{}\begingroup\count1=4 \endgroup
\begingroup\tracingassigns=0 \endgroup
\advance\count1 by 1 \multiply\count1 by 1 \divide\count1 by 1 \advance\dimen1 by 1pt \advance\skip1 by 0pt
\futurelet\a\relax\relax\read16 to\a
""")
P('tracinggroups', r"""
\T{tracinggroups}
\tracinggroups=1
{}\begingroup\endgroup{{}}
\setbox1=\hbox{}\setbox1=\vbox{}\setbox1=\vtop{}\setbox1=\hbox{$a^{b}\left(c\right)\mathchoice{}{}{}{}\vcenter{}$}
\setbox1=\hbox{\discretionary{}{}{}}\setbox1=\vbox{\halign{#\cr a\cr\noalign{}}}\setbox1=\hbox{\valign{#\cr\hbox{}\cr}}
\insert100{}\noindent\vadjust{}$a$\par $$a$$
\output={\setbox0=\box255 \deadcycles=0 }\hbox{}\penalty-10000
{\endgroup}
\begingroup}
\endgroup
\tracinggroups=0 {}
""")
P('tracingifs', r"""
\T{tracingifs}
\tracingifs=1
\iftrue\fi\iffalse\fi\iffalse\else\fi\ifnum1=1 \fi\ifdim1pt=1pt \fi\if aa\fi\ifcat aa\fi\ifx aa\fi\ifodd1 \fi
\ifvmode\fi\ifhmode\fi\ifmmode\fi\ifinner\fi\ifvoid0 \fi\ifhbox0 \fi\ifvbox0 \fi\ifeof0 \fi
\ifcase0 \or\fi\ifcase1 \or\fi\ifcase2 \or\else\fi
\unless\iftrue\else\fi\unless\iffalse\fi
\ifdefined\relax\fi\ifcsname relax\endcsname\fi\iffontchar\tenrm65 \fi\ifincsname\fi\ifpdfprimitive\relax\fi\ifpdfabsnum1=1 \fi\ifpdfabsdim1pt=1pt \fi
\iftrue\iffalse\else\fi\fi
\iffalse\if\fi\fi
\fi
\else
\or
\tracingifs=0 \iftrue\fi
""")
P('tracingscantokens', r"""
\T{tracingscantokens}
\tracingscantokens=1
\scantokens{\V{a}}
\scantokens{\V{one}
\V{two}}
\scantokens{}
\immediate\openout1=probetrace.tex \immediate\write1{\string\V{file}}\immediate\closeout1
\input probetrace.tex
\scantokens{\errmessage{inside}}
\tracingscantokens=0 \scantokens{\V{b}}
""")
P('tracingnesting', r"""
\T{tracingnesting}
\tracingnesting=1
\immediate\openout1=probenest.tex
\immediate\write1{\string\begingroup}
\immediate\write1{\string\iftrue}
\immediate\closeout1
\input probenest.tex
\endgroup\fi
\scantokens{\begingroup}\endgroup
\scantokens{\iftrue}\fi
\immediate\openout1=probenest2.tex
\immediate\write1{\string\endgroup}
\immediate\write1{\string\fi}
\immediate\closeout1
\begingroup\iftrue\input probenest2.tex
\tracingnesting=2
\input probenest.tex
\endgroup\fi
\scantokens{\begingroup}\endgroup
\scantokens{\iftrue}\fi
\begingroup\iftrue\input probenest2.tex
\tracingnesting=0 \input probenest.tex
\endgroup\fi
""")
P('tracingstacklevels', r"""
\T{tracingstacklevels}
\showthe\tracingstacklevels
\tracingmacros=2 \tracingstacklevels=1
\def\a{\b}\def\b{\c}\def\c{\d}\def\d{\relax}\a
\tracingstacklevels=2 \a
\tracingstacklevels=3 \a
\tracingstacklevels=100 \a
\tracingstacklevels=-1 \a
\tracingstacklevels=0 \a
\tracingstacklevels=2147483647 \a \setbox1=\hbox{\a$\a$}\showlists
\immediate\openout1=probestack.tex \immediate\write1{\string\a}\immediate\closeout1
\input probestack.tex
\tracingstacklevels=0 \tracingmacros=1
""")
P('partokenname partokencontext', r"""
\T{partoken}
\showthe\partokencontext
\show\partokenname
\partokenname\mypar \show\mypar \show\par
\def\mypar{\V{mypar}\endgraf}
\noindent a

\noindent b\mypar
\def\a#1{[#1]}\a{x

y}
\a{x\mypar y}
\V{\meaning\mypar}
\partokenname\par
\noindent c

\partokenname x
\partokenname
\relax
\partokenname\undefinedname \show\undefinedname
\partokenname\relax
\show\relax
\partokenname\par
\partokencontext=0 \showthe\partokencontext
\setbox1=\vbox{\noindent\hbox{a}\par\showlists}
\everypar={\V{ep}}\setbox1=\vbox{\vrule\noindent a\par\showlists}\everypar={}
\partokencontext=1 \showthe\partokencontext
\setbox1=\vbox{\noindent a\hrule\showlists}
\setbox1=\vbox{\noindent a\vskip 1pt\showlists}
\setbox1=\hbox{\vbox{\noindent a}}\showbox1
\partokencontext=2 \showthe\partokencontext
\def\par{\V{redef par ran}\endgraf}
\setbox1=\vbox{\noindent a\hrule}
\setbox1=\vbox{\noindent a}
\noindent a\vskip 1pt
\noindent a\vfill\penalty-10000
\let\par=\endgraf
\partokencontext=3 \partokencontext=-1 \partokencontext=2147483647 \showthe\partokencontext \partokencontext=0
""")
P('showstream', r"""
\T{showstream}
\showthe\showstream
\showstream=5 \show\relax
\immediate\openout5=probeshow.out
\showstream=5
\show\relax\showthe\count0 \showbox0 \showtokens{a}\showlists\showgroups\showifs
\def\a{b}\show\a
\showstream=-1
\show\relax
\immediate\closeout5
\showstream=5 \show\relax
\showstream=16 \show\relax\showstream=17 \show\relax
\showstream=2147483647 \show\relax\showstream=-2147483647 \show\relax
\showstream=-1
""")
P('synctex', r"""
\T{synctex}
\showthe\synctex
\synctex=0 \showthe\synctex
\begingroup\synctex=1 \showthe\synctex\endgroup\showthe\synctex
\synctex=-1 \showthe\synctex
\synctex=2147483647 \showthe\synctex
\synctex=0
""")
# ---------------------------------------------------------------------------------------------- pdfTeX conditionals and expansion
P('pdfprimitive ifpdfprimitive', r"""
\T{pdfprimitive}
\ifpdfprimitive\relax\V{a}\else\V{b}\fi\ifpdfprimitive\hbox\V{c}\else\V{d}\fi\ifpdfprimitive\pdfprimitive\V{e}\else\V{f}\fi
\def\m{}\ifpdfprimitive\m\V{g}\else\V{h}\fi\ifpdfprimitive\undefinedthing\V{i}\else\V{j}\fi
\ifpdfprimitive a\V{k}\else\V{l}\fi\ifpdfprimitive~\V{m}\else\V{n}\fi
\ifpdfprimitive\tenrm\V{o}\else\V{p}\fi\ifpdfprimitive\thinspace\V{q}\else\V{r}\fi
\ifpdfprimitive\fi
\expandafter\ifpdfprimitive\csname relax\endcsname\V{s}\else\V{t}\fi
\expandafter\ifpdfprimitive\csname nosuch\endcsname\V{u}\else\V{v}\fi
\begingroup\def\relax{x}\ifpdfprimitive\relax\V{w}\else\V{x}\fi
\pdfprimitive\relax\show\relax
\let\kern=\penalty\setbox1=\hbox{\kern5 \pdfprimitive\kern5pt}\showbox1
\ifpdfprimitive\kern\V{y}\else\V{z}\fi
\endgroup
\begingroup\let\hbox=\relax\ifpdfprimitive\hbox\V{1}\else\V{2}\fi\setbox1=\pdfprimitive\hbox{x}\showbox1\endgroup
\begingroup\let\relax=\kern\ifpdfprimitive\relax\V{3}\else\V{4}\fi\endgroup
\pdfprimitive\foo
\pdfprimitive a
\pdfprimitive\undefinedthing
\pdfprimitive~
\ignoreprimitiveerror=1 \pdfprimitive\foo \pdfprimitive x\par \ignoreprimitiveerror=0
\edef\a{\pdfprimitive\relax}\show\a
\edef\a{\pdfprimitive\number5}\show\a
\edef\a{\noexpand\pdfprimitive\relax}\show\a
\V{\pdfprimitive\number5}\V{\pdfprimitive\the\count0}
\expandafter\show\pdfprimitive\relax
\pdfprimitive\count1=5 \showthe\count1
\showthe\pdfprimitive\count1
\pdfprimitive\pdfprimitive\relax
\pdfprimitive\fi
\iftrue\pdfprimitive\fi
\iffalse\pdfprimitive\fi\fi
\let\p=\pdfprimitive\p\relax\show\p
\show\pdfprimitive\show\ifpdfprimitive
\ifpdfprimitive\else\V{5}\fi
""")
P('ignoreprimitiveerror', r"""
\T{ignoreprimitiveerror}
\showthe\ignoreprimitiveerror
\pdfprimitive\nosuchthing
\ignoreprimitiveerror=1 \showthe\ignoreprimitiveerror \pdfprimitive\nosuchthing \pdfprimitive x\par
\ignoreprimitiveerror=2 \pdfprimitive\nosuchthing \pdfprimitive y\par
\ignoreprimitiveerror=-1 \pdfprimitive\nosuchthing
\ignoreprimitiveerror=2147483647 \pdfprimitive\nosuchthing
\ignoreprimitiveerror=0
""")
P('ifincsname', r"""
\T{ifincsname}
\ifincsname\V{a}\else\V{b}\fi
\expandafter\show\csname\ifincsname yes\else no\fi\endcsname
\ifcsname\ifincsname q\else r\fi\endcsname\V{c}\else\V{d}\fi
\expandafter\show\csname\ifincsname\csname\ifincsname y\else n\fi\endcsname\fi\endcsname
\def\m{\ifincsname i\else o\fi}\expandafter\show\csname\m\endcsname\V{\m}
\edef\a{\m}\show\a
\show\ifincsname
\showifs
\ifincsname\fi
""")
P('ifpdfabsnum', r"""
\T{ifpdfabsnum}
\ifpdfabsnum1=1 \V{a}\else\V{b}\fi\ifpdfabsnum-1=1 \V{c}\else\V{d}\fi\ifpdfabsnum1=-1 \V{e}\else\V{f}\fi\ifpdfabsnum-1=-1 \V{g}\else\V{h}\fi
\ifpdfabsnum-2<1 \V{i}\else\V{j}\fi\ifpdfabsnum-1<2 \V{k}\else\V{l}\fi\ifpdfabsnum-2>1 \V{m}\else\V{n}\fi\ifpdfabsnum1>-2 \V{o}\else\V{p}\fi
\ifpdfabsnum-2147483647=2147483647 \V{q}\else\V{r}\fi
\ifpdfabsnum\count0=0 \V{s}\else\V{t}\fi
\ifpdfabsnum 1x2 \V{u}\else\V{v}\fi
\ifpdfabsnum x=1 \V{w}\else\V{x}\fi
\ifpdfabsnum 1=x \V{y}\else\V{z}\fi
\ifpdfabsnum 1 \V{1}\else\V{2}\fi
\ifpdfabsnum\fi
\show\ifpdfabsnum
""")
P('ifpdfabsdim', r"""
\T{ifpdfabsdim}
\ifpdfabsdim1pt=1pt \V{a}\else\V{b}\fi\ifpdfabsdim-1pt=1pt \V{c}\else\V{d}\fi\ifpdfabsdim1pt=-1pt \V{e}\else\V{f}\fi\ifpdfabsdim-1pt=-1pt \V{g}\else\V{h}\fi
\ifpdfabsdim-2pt<1pt \V{i}\else\V{j}\fi\ifpdfabsdim-1pt<2pt \V{k}\else\V{l}\fi\ifpdfabsdim-2pt>1pt \V{m}\else\V{n}\fi\ifpdfabsdim1pt>-2pt \V{o}\else\V{p}\fi
\ifpdfabsdim-16383.99999pt=\maxdimen \V{q}\else\V{r}\fi
\ifpdfabsdim\dimen0=0pt \V{s}\else\V{t}\fi
\ifpdfabsdim 1ptx2pt \V{u}\else\V{v}\fi
\ifpdfabsdim x=1pt \V{w}\else\V{x}\fi
\ifpdfabsdim 1pt=x \V{y}\else\V{z}\fi
\ifpdfabsdim 1pt \V{1}\else\V{2}\fi
\ifpdfabsdim\fi
\show\ifpdfabsdim
""")
P('expanded', r"""
\T{expanded}
\def\b{B}\V{\expanded{\b\noexpand\b}}\edef\a{\expanded{\b\noexpand\b}}\show\a
\edef\a{\noexpand\expanded{\b}}\show\a
\edef\a{\expanded{\unexpanded{\b}\b}}\show\a
\edef\a{\expanded{#}}\show\a
\edef\a{\expanded{##}}\show\a
\edef\a{\expanded{\expanded{\b}}}\show\a
\edef\a{\expanded\expandafter{\b}}\show\a
\expandafter\def\expanded{\noexpand\c\b}{d}\show\cB
\protected\def\p{P}\edef\a{\expanded{\p\b}}\show\a
\toks0={x#}\edef\a{\expanded{\the\toks0}}\show\a
\expandafter\show\expanded{\b}
\expanded{\relax}\par
\expanded x\par
\V{\expanded\relax{y}}
\setbox1=\hbox{\expanded{\b}}\showbox1
\V{\expanded{\undefinedthing}}
\V{\expanded{\iffalse}\fi}
\V{\expanded{\fi}}
\expanded{}}
\show\expanded
\edef\a{\expanded\toks0}\show\a
\expanded{\errmessage{in expanded}}
""")
# ---------------------------------------------------------------------------------------------- pdfTeX string functions
P('pdfstrcmp', r"""
\T{pdfstrcmp}
\V{\pdfstrcmp{a}{a}}\V{\pdfstrcmp{a}{b}}\V{\pdfstrcmp{b}{a}}\V{\pdfstrcmp{}{}}\V{\pdfstrcmp{}{a}}\V{\pdfstrcmp{a}{}}
\V{\pdfstrcmp{abc}{abd}}\V{\pdfstrcmp{abc}{ab}}\V{\pdfstrcmp{ab}{abc}}\V{\pdfstrcmp{A}{a}}\V{\pdfstrcmp{^^ff}{a}}\V{\pdfstrcmp{^^80}{^^7f}}
\def\m{a}\V{\pdfstrcmp{\m}{a}}\V{\pdfstrcmp{\noexpand\m}{\string\m}}\V{\pdfstrcmp{\noexpand\m}{a}}
\V{\pdfstrcmp{#}{##}}\V{\pdfstrcmp{\relax}{\string\relax}}
\V{\pdfstrcmp{ }{\space}}\V{\pdfstrcmp{a b}{a\space b}}
\catcode`\~=12 \V{\pdfstrcmp{~}{~}}\V{\pdfstrcmp{\detokenize{a b}}{a b}}
\V{\pdfstrcmp a{b}}\par
\V{\pdfstrcmp{a}b}\par
\pdfstrcmp{a}{a}\par
\edef\a{\pdfstrcmp{x}{y}}\show\a
\ifnum\pdfstrcmp{a}{a}=0 \V{eq}\fi
\V{\pdfstrcmp{\undefinedthing}{x}}
\show\pdfstrcmp
\expandafter\show\pdfstrcmp{}{}
\V{\pdfstrcmp\expandafter{\jobname}{doc}}
\V{\pdfstrcmp{xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx}{x}}
""")
P('pdfmatch pdflastmatch', r"""
\T{pdfmatch}
\def\r#1#2{\V{[\pdfmatch{#1}{#2}][\pdflastmatch0][\pdflastmatch1][\pdflastmatch2][\pdflastmatch3][\pdflastmatch-1][\pdflastmatch100]}}
\r{a}{a}\r{a}{b}\r{}{}\r{}{x}\r{a}{}\r{^a}{ba}\r{a$}{ab}\r{a|b}{b}\r{(a)(b)}{xaby}\r{(a|b)+}{abab}
\r{[a-c]+}{xxabcxx}\r{[^a-c]+}{abxyab}\r{a*}{aaa}\r{a+}{}\r{a?b}{b}\r{a{2,3}}{aaaa}\r{a{2}}{a}
\r{(a)|(b)}{b}\r{(a*)(b*)}{aabb}\r{(a*)*}{aa}\r{(a|ab)(c|bcd)}{abcd}\r{a.c}{abc}\r{a\string\.c}{a.c}
\r{[[:digit:]]+}{ab123}\r{[[:alpha:]]+}{12ab}\r{[[:space:]]}{a b}
\T{pdfmatch icase and subcount}
\V{\pdfmatch icase{A}{a}\pdflastmatch0}\V{\pdfmatch{A}{a}\pdflastmatch0}
\V{\pdfmatch subcount 0{(a)}{a}\pdflastmatch0|\pdflastmatch1}
\V{\pdfmatch subcount 1{(a)(b)}{ab}\pdflastmatch0|\pdflastmatch1|\pdflastmatch2}
\V{\pdfmatch subcount 2{(a)(b)}{ab}\pdflastmatch0|\pdflastmatch1|\pdflastmatch2}
\V{\pdfmatch subcount 100{(a)(b)}{ab}\pdflastmatch0|\pdflastmatch1|\pdflastmatch2}
\V{\pdfmatch icase subcount 2{(A)}{a}\pdflastmatch0}
\V{\pdfmatch subcount -1{(a)}{a}}
\V{\pdfmatch subcount x{(a)}{a}}
\T{pdfmatch bad patterns}
\V{\pdfmatch{(}{a}}\V{\pdfmatch{)}{a}}\V{\pdfmatch{[}{a}}\V{\pdfmatch{*}{a}}\V{\pdfmatch{a{2,1}}{a}}\V{\pdfmatch{\string\\}{a}}\V{\pdfmatch{a**}{a}}
\V{[\pdflastmatch0]}
\pdfmatch{a}{a}\par
\pdflastmatch0\par
\V{\pdfmatch a{a}}\par
\V{\pdfmatch{a}a}\par
\V{\pdflastmatch x}\par
\V{\pdflastmatch 2147483648}\par
\edef\a{\pdfmatch{a}{a}\pdflastmatch0}\show\a
\def\p{a+}\def\s{caab}\V{\pdfmatch{\p}{\s}\pdflastmatch0}
\show\pdfmatch\show\pdflastmatch
\V{\pdfmatch{^^80}{^^80}\pdflastmatch0}
\V{\pdfmatch{ }{a b}\pdflastmatch0}
""")
P('pdfescapehex pdfunescapehex', r"""
\T{pdfescapehex}
\V{\pdfescapehex{}}\V{\pdfescapehex{a}}\V{\pdfescapehex{abc}}\V{\pdfescapehex{ }}\V{\pdfescapehex{^^00^^ff^^80}}
\def\m{x}\V{\pdfescapehex{\m}}\V{\pdfescapehex{\noexpand\m}}\V{\pdfescapehex{#}}\V{\pdfescapehex{\relax}}
\V{\pdfescapehex a}\par
\pdfescapehex{a}\par
\edef\a{\pdfescapehex{a}}\show\a
\T{pdfunescapehex}
\V{\pdfunescapehex{}}\V{\pdfunescapehex{41}}\V{\pdfunescapehex{414243}}\V{\pdfunescapehex{4}}\V{\pdfunescapehex{414}}
\V{\pdfunescapehex{4a4A}}\V{\pdfunescapehex{zz}}\V{\pdfunescapehex{4 1}}\V{\pdfunescapehex{41zz42}}\V{\pdfunescapehex{4g}}\V{\pdfunescapehex{g4}}
\V{\pdfunescapehex{20}}\V{\pdfunescapehex{00}}\V{\pdfunescapehex{ff}}\V{\pdfunescapehex{0a}}\V{\pdfunescapehex{0d}}\V{\pdfunescapehex{7e}}\V{\pdfunescapehex{7f}}\V{\pdfunescapehex{80}}
\V{\pdfunescapehex\expandafter{\pdfescapehex{a b}}}
\edef\a{\pdfunescapehex{41}}\show\a
\edef\a{\pdfunescapehex{20}}\show\a
\edef\a{\pdfunescapehex{5c}}\show\a
\V{\pdfunescapehex a}\par
\pdfunescapehex{41}\par
\show\pdfescapehex\show\pdfunescapehex
\expandafter\show\pdfunescapehex{41}
""")
P('pdfescapename', r"""
\T{pdfescapename}
\V{\pdfescapename{}}\V{\pdfescapename{abc}}\V{\pdfescapename{a b}}\V{\pdfescapename{a#b}}\V{\pdfescapename{a/b}}
\V{\pdfescapename{()<>[]{}}}\V{\pdfescapename{\pc}}\V{\pdfescapename{^^00^^1f^^7f^^80^^ff}}\V{\pdfescapename{!"$&'*+,-.:;=?@\string\^_`|~}}
\def\m{x}\V{\pdfescapename{\m}}\V{\pdfescapename{\noexpand\m}}
\V{\pdfescapename a}\par
\pdfescapename{a}\par
\edef\a{\pdfescapename{a b}}\show\a
\show\pdfescapename
""")
P('pdfescapestring', r"""
\T{pdfescapestring}
\V{\pdfescapestring{}}\V{\pdfescapestring{abc}}\V{\pdfescapestring{a b}}\V{\pdfescapestring{(a)}}\V{\pdfescapestring{\string\\}}
\V{\pdfescapestring{^^00^^08^^09^^0a^^0c^^0d^^1f^^7f^^80^^ff}}\V{\pdfescapestring{<>[]/#\pc}}
\def\m{x}\V{\pdfescapestring{\m}}\V{\pdfescapestring{\noexpand\m}}\V{\pdfescapestring{\string\m}}
\V{\pdfescapestring a}\par
\pdfescapestring{a}\par
\edef\a{\pdfescapestring{(a b)}}\show\a
\show\pdfescapestring
""")
P('pdfmdfivesum', r"""
\T{pdfmdfivesum}
\V{\pdfmdfivesum{}}\V{\pdfmdfivesum{a}}\V{\pdfmdfivesum{abc}}\V{\pdfmdfivesum{message digest}}
\V{\pdfmdfivesum{12345678901234567890123456789012345678901234567890123456789012345678901234567890}}
\V{\pdfmdfivesum{^^00^^ff}}\def\m{x}\V{\pdfmdfivesum{\m}}\V{\pdfmdfivesum{\noexpand\m}}
\V{\pdfmdfivesum file {plain.tex}}\V{\pdfmdfivesum file {nosuchfile.xyz}}\V{\pdfmdfivesum file {}}
\V{\pdfmdfivesum file {\jobname.tex}}
\V{\pdfmdfivesum a}\par
\pdfmdfivesum{a}\par
\V{\pdfmdfivesum file a}\par
\edef\a{\pdfmdfivesum{a}}\show\a
\show\pdfmdfivesum
""")
P('pdffilesize pdffilemoddate pdffiledump', r"""
\T{file functions}
\V{[\pdffilesize{plain.tex}]}\V{[\pdffilesize{nosuchfile.xyz}]}\V{[\pdffilesize{}]}\V{[\pdffilesize{\jobname.tex}]}
\V{[\pdffilemoddate{plain.tex}]}\V{[\pdffilemoddate{nosuchfile.xyz}]}\V{[\pdffilemoddate{}]}
\V{[\pdffiledump{plain.tex}]}
\V{[\pdffiledump offset 0 length 16{plain.tex}]}\V{[\pdffiledump length 16{plain.tex}]}\V{[\pdffiledump offset 5 length 3{plain.tex}]}
\V{[\pdffiledump offset 1000000000 length 3{plain.tex}]}\V{[\pdffiledump length 0{plain.tex}]}\V{[\pdffiledump offset 0 length 16{nosuchfile.xyz}]}
\V{[\pdffiledump length -1{plain.tex}]}\V{[\pdffiledump offset -1 length 3{plain.tex}]}\V{[\pdffiledump length x{plain.tex}]}
\V{\pdffilesize a}\par
\V{\pdffilemoddate a}\par
\V{\pdffiledump a}\par
\pdffilesize{plain.tex}\par
\pdffilemoddate{plain.tex}\par
\pdffiledump length 2{plain.tex}\par
\edef\a{\pdffilesize{plain.tex}\pdffilemoddate{plain.tex}\pdffiledump length 2{plain.tex}}\show\a
\show\pdffilesize\show\pdffilemoddate\show\pdffiledump
\def\n{plain.tex}\V{\pdffilesize{\n}}
""")
P('pdfcreationdate', r"""
\T{pdfcreationdate}
\V{\pdfcreationdate}\edef\a{\pdfcreationdate}\show\a
\pdfcreationdate\par
\expandafter\show\pdfcreationdate
\show\pdfcreationdate
\V{\the\year/\the\month/\the\day/\the\time}
""")
P('pdftexbanner pdftexrevision pdftexversion', r"""
\T{pdfTeX version strings}
\V{\pdftexbanner}\V{\pdftexrevision}\V{\the\pdftexversion}\showthe\pdftexversion
\edef\a{\pdftexbanner}\show\a\edef\a{\pdftexrevision}\show\a
\pdftexbanner\par
\pdftexrevision\par
\pdftexversion\par
\pdftexversion=1
\expandafter\show\pdftexrevision
""")
P('pdfshellescape', r"""
\T{pdfshellescape}
\V{\the\pdfshellescape}\showthe\pdfshellescape
\pdfshellescape=1
\pdfshellescape\par
\immediate\write18{echo hi}
\immediate\write18{}
\write18{echo delayed}\shipout\hbox{\write18{echo in box}}
\V{after}
""")
P('pdfuniformdeviate pdfnormaldeviate pdfsetrandomseed pdfrandomseed', r"""
\T{random numbers}
\pdfsetrandomseed 42 \V{\the\pdfrandomseed}
\V{\pdfuniformdeviate 100}\V{\pdfuniformdeviate 100}\V{\pdfuniformdeviate 2147483647}\V{\pdfuniformdeviate -5}\V{\pdfuniformdeviate 0}\V{\pdfuniformdeviate 1}
\V{\pdfnormaldeviate}\V{\pdfnormaldeviate}\V{\pdfnormaldeviate}
\pdfsetrandomseed 0 \V{\the\pdfrandomseed}\V{\pdfuniformdeviate 1000}\V{\pdfnormaldeviate}
\pdfsetrandomseed -1 \V{\the\pdfrandomseed}\V{\pdfuniformdeviate 1000}\V{\pdfnormaldeviate}
\pdfsetrandomseed 2147483647 \V{\the\pdfrandomseed}\V{\pdfuniformdeviate 1000}\V{\pdfnormaldeviate}
\pdfsetrandomseed 2147483648
\pdfsetrandomseed x
\pdfsetrandomseed 1 \count1=0 \loop\ifnum\count1<60 \message{\pdfuniformdeviate 1000000}\advance\count1 by 1 \repeat
\count1=0 \loop\ifnum\count1<60 \message{\pdfnormaldeviate}\advance\count1 by 1 \repeat
\V{\pdfuniformdeviate x}\par
\pdfuniformdeviate 5\par
\pdfnormaldeviate\par
\pdfrandomseed\par
\pdfrandomseed=1
\edef\a{\pdfuniformdeviate 5\pdfnormaldeviate}\show\a
\show\pdfuniformdeviate\show\pdfnormaldeviate\show\pdfsetrandomseed\show\pdfrandomseed
\setbox1=\hbox{\pdfsetrandomseed 7 }\showbox1
\begingroup\pdfsetrandomseed 8 \endgroup\V{\the\pdfrandomseed}
\global\pdfsetrandomseed 9 \V{\the\pdfrandomseed}
\immediate\pdfsetrandomseed 10 \V{\the\pdfrandomseed}
""")
P('pdfelapsedtime pdfresettimer', r"""
\T{timer}
\pdfresettimer
\ifnum\pdfelapsedtime<100000000 \V{time is small}\else\V{time is large}\fi
\ifnum\pdfelapsedtime<0 \V{negative}\else\V{not negative}\fi
\pdfelapsedtime\par
\pdfelapsedtime=1
\setbox1=\hbox{\pdfresettimer}\showbox1
\pdfresettimer x\par
\show\pdfresettimer\show\pdfelapsedtime
\global\pdfresettimer
\immediate\pdfresettimer
""")
# ---------------------------------------------------------------------------------------------- PDF objects
PAGE = r"""
\def\pg#1{\shipout\vbox{\hsize=200pt \noindent\hbox to 100pt{#1\hfil}}}
"""
P('pdfobj pdflastobj pdfrefobj', PAGE + r"""
\T{pdfobj}
\showthe\pdflastobj
\pdfobj {<</A 1>>}\showthe\pdflastobj\pdfrefobj\pdflastobj
\immediate\pdfobj {[1 2 3]}\showthe\pdflastobj
\pdfobj stream {stream data}\showthe\pdflastobj\pdfrefobj\pdflastobj
\pdfobj stream attr {/X 1} {with attr}\showthe\pdflastobj\pdfrefobj\pdflastobj
\immediate\pdfobj stream {immediate stream}\showthe\pdflastobj
\pdfobj file {plain.tex}\showthe\pdflastobj\pdfrefobj\pdflastobj
\pdfobj stream file {plain.tex}\showthe\pdflastobj\pdfrefobj\pdflastobj
\pdfobj reserveobjnum \count1=\pdflastobj\showthe\count1
\pdfobj useobjnum \count1 {(filled)}\showthe\pdflastobj\pdfrefobj\count1
\immediate\pdfobj reserveobjnum \count2=\pdflastobj
\immediate\pdfobj useobjnum \count2 {(filled immediately)}
\pdfobj uncompressed {/U 1}\pdfrefobj\pdflastobj
\immediate\pdfobj uncompressed {/U 2}
\def\m{\the\count0}\pdfobj {(\m)}\pdfrefobj\pdflastobj
\setbox1=\hbox{\pdfrefobj\pdflastobj}\showbox1
\setbox1=\vbox{\pdfrefobj\pdflastobj}\showbox1
\setbox1=\hbox{$\pdfrefobj\pdflastobj$}\showbox1
\pg{\pdfobj {(page level)}\pdfrefobj\pdflastobj}
\pdfobj useobjnum 0 {x}
\pdfobj useobjnum 1000000 {x}
\pdfrefobj 0
\pdfrefobj 1000000
\pdfrefobj x\par
\pdfobj x\par
\pdfobj stream attr x\par
\pdfobj
\par
\pdflastobj=1
\pdflastobj
\show\pdfobj\show\pdfrefobj\show\pdflastobj
""")
P('pdfxform pdfrefxform pdflastxform pdfxformname', PAGE + r"""
\T{pdfxform}
\showthe\pdflastxform
\setbox1=\hbox{xform one}\pdfxform 1 \showthe\pdflastxform\showbox1
\pg{\pdfrefxform\pdflastxform\pdfrefxform\pdflastxform}
\V{\pdfxformname\pdflastxform}
\setbox1=\vbox{\hbox{v}\hrule}\immediate\pdfxform 1 \count1=\pdflastxform\showthe\count1\showbox1
\setbox1=\hbox to 50pt{a\hfil b}\pdfxform attr {/Foo 1} resources {/ProcSet [/PDF]} 1 \count2=\pdflastxform
\setbox1=\hbox{\special{x}}\immediate\pdfxform attr {/Bar 2} 1 \count3=\pdflastxform
\setbox1=\hbox{\setbox2=\hbox{inner}\pdfxform 2 \pdfrefxform\pdflastxform}\pdfxform 1 \count4=\pdflastxform
\pg{\pdfrefxform\count1 \pdfrefxform\count2 \pdfrefxform\count3 \pdfrefxform\count4 }
\setbox1=\hbox{a}\wd1=100pt \ht1=20pt \dp1=5pt \pdfxform 1 \count5=\pdflastxform
\setbox1=\hbox{\pdfrefxform\count5}\showbox1 \V{\the\wd1 \the\ht1 \the\dp1}\shipout\box1
\setbox1=\vbox{\pdfrefxform\count5}\showbox1 \shipout\box1
\setbox1=\hbox{$\pdfrefxform\count5$}\showbox1
\setbox1=\hbox{\raise 3pt\hbox{\pdfrefxform\count5}}\shipout\box1
\pdfrefxform 0
\pdfrefxform 1000000
\pdfrefxform x
\pdfrefxform\count5\par
\V{\pdfxformname 0}\V{\pdfxformname 1000000}\V{\pdfxformname x}\par
\pdfxformname\count5\par
\pdflastxform=1
\pdflastxform
\show\pdfxform\show\pdfrefxform\show\pdflastxform\show\pdfxformname
""")
P('pdfximage pdfrefximage pdflastximage pdflastximagepages pdflastximagecolordepth pdfximagebbox', PAGE + r"""
\T{pdfximage}
\showthe\pdflastximage\showthe\pdflastximagepages\showthe\pdflastximagecolordepth
\pdfximage {probe.jpg}\showthe\pdflastximage\showthe\pdflastximagepages\showthe\pdflastximagecolordepth
\pg{\pdfrefximage\pdflastximage}
\V{\pdfximagebbox\pdflastximage1,\pdfximagebbox\pdflastximage2,\pdfximagebbox\pdflastximage3,\pdfximagebbox\pdflastximage4}
\V{\pdfximagebbox\pdflastximage0}\V{\pdfximagebbox\pdflastximage5}\V{\pdfximagebbox 0 1}
\pdfximage width 1cm {probe.jpg}\pg{\pdfrefximage\pdflastximage}
\pdfximage height 1cm {probe.jpg}\pg{\pdfrefximage\pdflastximage}
\pdfximage depth 1pt height 1cm width 2cm {probe.jpg}\pg{\pdfrefximage\pdflastximage}
\immediate\pdfximage width 2cm {probe.jpg}\count1=\pdflastximage\pg{\pdfrefximage\count1 \pdfrefximage\count1 }
\pdfximage attr {/Foo 1} {probe.jpg}\pg{\pdfrefximage\pdflastximage}
\setbox1=\hbox{\pdfrefximage\count1}\showbox1\V{\the\wd1 \the\ht1 \the\dp1}
\setbox1=\vbox{\pdfrefximage\count1}\showbox1
\setbox1=\hbox{$\pdfrefximage\count1$}\showbox1
\begingroup\pdfimageresolution=300 \pdfximage {probe.jpg}\pg{\pdfrefximage\pdflastximage}\endgroup
\pdfrefximage 0
\pdfrefximage 1000000
\pdfrefximage x
\pdfrefximage\count1\par
\pdflastximage=1
\pdflastximage
\pdflastximagepages
\pdflastximagecolordepth
\pdfximagebbox\count1 1\par
\show\pdfximage\show\pdfrefximage\show\pdflastximage\show\pdflastximagepages\show\pdflastximagecolordepth\show\pdfximagebbox
""")
P('pdfannot pdflastannot', PAGE + r"""
\T{pdfannot}
\showthe\pdflastannot
\def\an#1{\pg{\pdfannot #1 {/Subtype /Text /Contents (a)}\V{\the\pdflastannot}}}
\an{}\an{width 10pt}\an{height 10pt}\an{depth 3pt}\an{width 10pt height 10pt depth 3pt}\an{depth 3pt width 10pt}
\pdfannot reserveobjnum \count1=\pdflastannot\showthe\count1
\pg{\pdfannot useobjnum \count1 width 5pt {/Subtype /Text}}
\pg{\pdfannot {/Subtype /Text}\pdfannot {/Subtype /Text}\kern 10pt\pdfannot {/Subtype /Text}}
\pg{\pdflinkmargin=5pt \pdfannot width 5pt height 5pt {/Subtype /Text}}
\setbox1=\hbox{\pdfannot width 1pt {/A 1}}\showbox1\shipout\box1
\setbox1=\vbox{\pdfannot width 1pt {/A 2}}\showbox1\shipout\box1
\setbox1=\hbox{$\pdfannot width 1pt {/A 3}$}\showbox1\shipout\box1
\pdfannot {/A 4}\hbox{}\vfill\penalty-10000
\pdfannot x\par
\pdfannot width x{}\par
\pdfannot useobjnum 0 {}\par
\pdfannot useobjnum 1000000 {}\par
\pdflastannot=1
\pdflastannot
\show\pdfannot\show\pdflastannot
\pdfannot reserveobjnum \pdfannot useobjnum \pdflastannot width 1pt {/B 1}\hbox{}\vfill\penalty-10000
""")
P('pdfstartlink pdfendlink pdflastlink pdflinkmargin pdfrunninglinkoff pdfrunninglinkon', PAGE + r"""
\T{links}
\showthe\pdflastlink\showthe\pdflinkmargin
\pdfdest name {t} fit \hbox{}\vfill\penalty-10000
\def\ln#1{\shipout\hbox{\pdfstartlink #1 x\pdfendlink}\V{\the\pdflastlink}}
\ln{goto page 1 {/Fit}}\ln{goto name {t}}\ln{goto num 1}\ln{user {/Subtype /Link /A << /S /URI /URI (u) >>}}
\ln{attr {/C [1 0 0]} goto page 1 {/Fit}}\ln{width 10pt height 10pt depth 3pt goto page 1 {/Fit}}\ln{width 10pt goto name {t}}
\ln{goto file {a.pdf} page 1 {/Fit}}\ln{goto file {a.pdf} name {b}}\ln{goto file {a.pdf} name {b} newwindow}\ln{goto file {a.pdf} name {b} nonewwindow}
\ln{goto struct {x} page 1 {/Fit}}\ln{thread page 1 {x}}\ln{thread name {t}}\ln{thread num 1}\ln{thread file {a.pdf} name {t}}
\ln{goto name {missingdest}}\ln{goto page 0 {/Fit}}
\begingroup\pdflinkmargin=5pt \ln{goto page 1 {/Fit}}\endgroup
\shipout\vbox{\hsize=60pt \noindent\pdfstartlink goto page 1 {/Fit} one two three four five six seven eight nine\pdfendlink\par}
\shipout\vbox{\hsize=60pt \noindent\pdfstartlink goto page 1 {/Fit} one two \pdfrunninglinkoff three four five \pdfrunninglinkon six seven eight nine\pdfendlink\par}
\shipout\hbox{\pdfstartlink goto page 1 {/Fit} a\hbox{\pdfrunninglinkoff b\pdfrunninglinkon c}d\pdfendlink}
\shipout\hbox{\vbox{\hbox{\pdfstartlink goto page 1 {/Fit} x}}\pdfendlink}
\shipout\hbox{\pdfstartlink goto page 1 {/Fit} a\pdfstartlink goto page 1 {/Fit} b\pdfendlink}
\setbox1=\hbox{\pdfendlink}\showbox1
\setbox1=\hbox{\pdfstartlink goto page 1 {/Fit}}\showbox1
\setbox1=\hbox{\pdfrunninglinkoff\pdfrunninglinkon}\showbox1
\setbox1=\hbox{$\pdfstartlink goto page 1 {/Fit} a\pdfendlink$}\showbox1
\pdfstartlink goto page 1 {/Fit}
\noindent\pdfstartlink goto page 1 {/Fit} x\pdfendlink\par
\pdfendlink
\pdfrunninglinkoff
\pdfrunninglinkon
\setbox1=\hbox{\pdfstartlink x}
\setbox1=\hbox{\pdfstartlink goto x}
\setbox1=\hbox{\pdfstartlink goto page x}
\setbox1=\hbox{\pdfstartlink goto name x}
\setbox1=\hbox{\pdfstartlink user x}
\pdflastlink=1
\pdflastlink
\pdflinkmargin=-1pt \showthe\pdflinkmargin\pdflinkmargin=0pt
\show\pdfstartlink\show\pdfendlink\show\pdflastlink\show\pdflinkmargin\show\pdfrunninglinkoff\show\pdfrunninglinkon
""")
P('pdfdest pdfdestmargin', PAGE + r"""
\T{dests}
\showthe\pdfdestmargin
\def\d#1{\shipout\hbox{\pdfdest #1 x}}
\d{name {a} xyz}\d{name {b} xyz zoom 1000}\d{name {c} xyz zoom 0}\d{name {d} fit}\d{name {e} fith}\d{name {f} fitv}\d{name {g} fitb}\d{name {h} fitbh}\d{name {i} fitbv}
\d{name {j} fitr}\d{name {k} fitr width 5pt}\d{name {l} fitr height 5pt depth 1pt}\d{name {m} fitr width 5pt height 5pt depth 1pt}
\d{num 100 fit}\d{num 101 xyz}\d{num 1 fit}
\d{struct name {s} fit}\d{struct num 5 fit}
\d{name {dup} fit}\d{name {dup} fit}\d{num 100 fit}
\begingroup\pdfdestmargin=5pt \d{name {n} fitr}\d{name {o} xyz}\d{name {p} fith}\endgroup
\d{name {} fit}\d{name {(paren)} fit}\d{name {with space} fit}\d{name {^^ff} fit}
\pdfsuppresswarningdupdest=1 \d{name {dup} fit}\pdfsuppresswarningdupdest=0
\setbox1=\hbox{\pdfdest name {q} fit}\showbox1\shipout\box1
\setbox1=\vbox{\pdfdest name {r} fit}\showbox1\shipout\box1
\setbox1=\hbox{$\pdfdest name {s} fit$}\showbox1\shipout\box1
\setbox1=\hbox{\pdfdest x}
\setbox1=\hbox{\pdfdest name x}
\setbox1=\hbox{\pdfdest num x}
\setbox1=\hbox{\pdfdest num 0 fit}
\setbox1=\hbox{\pdfdest num -1 fit}
\setbox1=\hbox{\pdfdest name {t} foo}
\setbox1=\hbox{\pdfdest name {t} xyz zoom x}
\pdfdest name {vertical} fit
\hbox{}\vfill\penalty-10000
\noindent\pdfdest name {horizontal} fit x\par
\vfill\penalty-10000
\pdfdestmargin=-1pt \showthe\pdfdestmargin \pdfdestmargin=0pt
\show\pdfdest\show\pdfdestmargin
""")
P('pdfoutline', PAGE + r"""
\T{outlines}
\pdfdest name {a} fit \hbox{}\vfill\penalty-10000
\pdfoutline goto name {a} {First}
\pdfoutline goto name {a} count 2 {Second}
\pdfoutline goto page 1 {/Fit} {Child one}
\pdfoutline goto page 1 {/Fit} count -1 {Child two}
\pdfoutline goto num 1 {Grandchild}
\pdfoutline user {/S /URI /URI (u)} {User}
\pdfoutline attr {/C [1 0 0]} goto name {a} {Attr}
\pdfoutline goto file {a.pdf} page 1 {/Fit} {File}
\pdfoutline goto file {a.pdf} name {b} {FileName}
\pdfoutline goto file {a.pdf} name {b} newwindow {NewWindow}
\pdfoutline goto name {missing} {Missing dest}
\pdfoutline goto name {a} {(parens) and \string\ backslash}
\pdfoutline goto name {a} {^^80^^ff non ascii}
\pdfoutline goto name {a} {}
\pdfoutline goto name {a} count 0 {Zero}
\pdfoutline goto name {a} count 100 {Too many children}
\setbox1=\hbox{\pdfoutline goto name {a} {In box}}\showbox1
\setbox1=\hbox{$\pdfoutline goto name {a} {In math}$}\showbox1
\immediate\pdfoutline goto name {a} {Immediate}
\global\pdfoutline goto name {a} {Global}
\pdfoutline x\par
\pdfoutline goto x\par
\pdfoutline goto name x\par
\pdfoutline goto name {a} count x\par
\pdfoutline goto name {a} x\par
\pdfoutline user x\par
\show\pdfoutline
""")
P('pdfthread pdfstartthread pdfendthread pdfthreadmargin', PAGE + r"""
\T{threads}
\showthe\pdfthreadmargin
\shipout\vbox{\hsize=100pt \pdfstartthread name {t} \noindent text in a thread\par\pdfendthread}
\shipout\vbox{\hsize=100pt \pdfstartthread num 7 \noindent another\par\pdfendthread}
\shipout\vbox{\hsize=100pt \pdfstartthread name {t} width 50pt height 10pt depth 2pt \noindent more\par\pdfendthread}
\shipout\vbox{\hsize=100pt \pdfthread name {u} \noindent one bead\par}
\shipout\vbox{\hsize=100pt \pdfthread num 8 width 10pt height 10pt depth 1pt \hbox{x}}
\shipout\vbox{\hsize=100pt \pdfthread attr {/Title (T)} name {v} \hbox{y}}
\shipout\vbox{\hsize=100pt \pdfstartthread attr {/Title (S)} name {w} \hbox{y}\pdfendthread}
\begingroup\pdfthreadmargin=3pt \shipout\vbox{\pdfthread name {u} \hbox{z}}\endgroup
\shipout\vbox{\hsize=60pt \pdfstartthread name {s} \vbox to 800pt{}\vbox to 800pt{}\pdfendthread}
\shipout\hbox{\pdfstartthread name {h}}
\shipout\vbox{\pdfendthread}
\shipout\vbox{\pdfstartthread name {g}\pdfstartthread name {g2}\hbox{}\pdfendthread}
\setbox1=\hbox{\pdfthread name {b}}\showbox1
\setbox1=\vbox{\pdfthread name {b}\pdfstartthread name {b}\pdfendthread}\showbox1
\setbox1=\hbox{$\pdfthread name {b}$}\showbox1
\setbox1=\hbox{\pdfthread x}
\setbox1=\hbox{\pdfthread name x}
\setbox1=\hbox{\pdfthread num x}
\setbox1=\hbox{\pdfstartthread x}
\pdfthread name {main}
\pdfstartthread name {main}
\pdfendthread
\pdfthreadmargin=-1pt \showthe\pdfthreadmargin \pdfthreadmargin=0pt
\show\pdfthread\show\pdfstartthread\show\pdfendthread\show\pdfthreadmargin
""")
P('pdfliteral', PAGE + r"""
\T{pdfliteral}
\pg{\pdfliteral{q 1 0 0 rg Q}}\pg{\pdfliteral direct{q Q}}\pg{\pdfliteral page{q Q}}\pg{\pdfliteral shipout{q Q}}
\pg{\pdfliteral{}}\pg{\pdfliteral direct{}}\pg{\pdfliteral page{}}
\def\m{\the\count0}\pg{\pdfliteral{\m\space 0 0 rg}}
\setbox1=\hbox{\pdfliteral{a}\pdfliteral direct{b}\pdfliteral page{c}}\showbox1
\setbox1=\vbox{\pdfliteral{d}}\showbox1\shipout\copy1
\setbox1=\hbox{$\pdfliteral{e}$}\showbox1
\pdfliteral{f}
\hbox{}\vfill\penalty-10000
\noindent\pdfliteral{g}x\par\vfill\penalty-10000
\pdfliteral x\par
\pdfliteral direct x\par
\pdfliteral foo{y}\par
\pdfliteral{}}
\show\pdfliteral
\setbox1=\hbox{\pdfliteral{\relax\string\a #}}\showbox1
\immediate\pdfliteral{imm}
\global\pdfliteral{glob}
""")
P('pdfcolorstack pdfcolorstackinit', PAGE + r"""
\T{colorstack}
\V{\pdfcolorstackinit page direct{0 g 0 G}}\V{\pdfcolorstackinit direct{1 g}}\V{\pdfcolorstackinit page {0 g}}\V{\pdfcolorstackinit {}}
\edef\a{\pdfcolorstackinit page direct{0.5 g}}\show\a
\chardef\c=\pdfcolorstackinit page direct{0 g}
\pg{\pdfcolorstack\c push{1 0 0 rg}x\pdfcolorstack\c pop y}
\pg{\pdfcolorstack\c set{0 1 0 rg}x\pdfcolorstack\c current y}
\pg{\pdfcolorstack\c push{a}\pdfcolorstack\c push{b}\pdfcolorstack\c push{c}x\pdfcolorstack\c pop\pdfcolorstack\c pop\pdfcolorstack\c pop}
\pg{\pdfcolorstack\c pop}
\pg{\pdfcolorstack\c current}
\pg{\pdfcolorstack\c push{across page}}
\pg{x}
\pg{\pdfcolorstack\c pop}
\pg{\pdfcolorstack 0 push{1 g}\pdfcolorstack 1 push{2 g}\pdfcolorstack 2 push{3 g}x\pdfcolorstack 0 pop\pdfcolorstack 1 pop\pdfcolorstack 2 pop}
\setbox1=\hbox{\pdfcolorstack\c push{p}\pdfcolorstack\c pop}\showbox1
\setbox1=\vbox{\pdfcolorstack\c set{s}}\showbox1
\setbox1=\hbox{$\pdfcolorstack\c current$}\showbox1
\pdfcolorstack\c push{1 g}\hbox{}\pdfcolorstack\c pop\vfill\penalty-10000
\immediate\pdfcolorstack\c pop
\V{\pdfcolorstackinit x}\par
\V{\pdfcolorstackinit page x}\par
\V{\pdfcolorstackinit direct x}\par
\pdfcolorstack 100 push{x}
\pdfcolorstack -1 push{x}
\pdfcolorstack x push{y}
\pdfcolorstack\c foo{y}\par
\pdfcolorstack\c push x\par
\pdfcolorstack\c set x\par
\pdfcolorstackinit page direct{}\par
\show\pdfcolorstack\show\pdfcolorstackinit
""")
P('pdfsetmatrix pdfsave pdfrestore', PAGE + r"""
\T{matrix save restore}
\pg{\pdfsave\pdfsetmatrix{1 0 0 1}x\pdfrestore}
\pg{\pdfsave\pdfsetmatrix{0.5 0 0 0.5}x\pdfsave\pdfsetmatrix{0 1 -1 0}y\pdfrestore\pdfrestore}
\pg{\pdfsetmatrix{1 0 0 1}}
\pg{\pdfsetmatrix{}}
\pg{\pdfsetmatrix{bad}}
\pg{\pdfsetmatrix{1 2 3}}
\pg{\pdfsetmatrix{1 2 3 4 5}}
\pg{\pdfsetmatrix{0 0 0 0}}
\pg{\pdfrestore}
\pg{\pdfsave}
\pg{\pdfsave\kern5pt\pdfrestore}
\pg{\pdfsave\hbox{\pdfrestore}}
\pg{\hbox{\pdfsave}\pdfrestore}
\pg{\hbox{\kern 3pt\pdfsave\kern 2pt\pdfrestore\kern 1pt}}
\shipout\vbox{\pdfsave\hbox{a}\pdfrestore}
\shipout\vbox{\kern 5pt\pdfsave\kern 3pt\pdfsetmatrix{2 0 0 2}\pdfrestore}
\setbox1=\hbox{\pdfsave\pdfsetmatrix{1 0 0 1}\pdfrestore}\showbox1
\setbox1=\vbox{\pdfsave\pdfsetmatrix{1 0 0 1}\pdfrestore}\showbox1
\setbox1=\hbox{$\pdfsave\pdfsetmatrix{1 0 0 1}\pdfrestore$}\showbox1
\pdfsave\pdfsetmatrix{1 0 0 1}\pdfrestore
\hbox{}\vfill\penalty-10000
\pdfsetmatrix x\par
\def\m{1 0 0 1}\pg{\pdfsetmatrix{\m}}
\show\pdfsetmatrix\show\pdfsave\show\pdfrestore
\immediate\pdfsave
\global\pdfrestore
""")
P('pdfsavepos pdflastxpos pdflastypos', PAGE + r"""
\T{savepos}
\showthe\pdflastxpos\showthe\pdflastypos
\def\w{\write-1{pos \the\pdflastxpos,\the\pdflastypos}}
\pg{\pdfsavepos\w}\pg{\kern 10pt\pdfsavepos\w\kern 10pt\pdfsavepos\w}
\shipout\vbox{\kern 10pt\pdfsavepos\w\kern 10pt\pdfsavepos\w}
\shipout\hbox{\raise 3pt\hbox{\pdfsavepos\w}\lower 3pt\hbox{\pdfsavepos\w}}
\showthe\pdflastxpos\showthe\pdflastypos
\begingroup\pdfhorigin=0pt \pdfvorigin=0pt \pg{\pdfsavepos\w}\endgroup
\begingroup\pdfpageheight=100pt \pdfpagewidth=100pt \pg{\pdfsavepos\w}\endgroup
\begingroup\hoffset=1pt \voffset=2pt \pg{\pdfsavepos\w}\endgroup
\begingroup\mag=2000 \endgroup
\setbox1=\hbox{\pdfsavepos}\showbox1
\setbox1=\vbox{\pdfsavepos}\showbox1
\setbox1=\hbox{$\pdfsavepos$}\showbox1
\pdfsavepos
\hbox{}\vfill\penalty-10000
\pdflastxpos=1
\pdflastypos=1
\pdflastxpos
\pdflastypos
\immediate\pdfsavepos\showthe\pdflastxpos
\show\pdfsavepos\show\pdflastxpos\show\pdflastypos
""")
P('pdfsnaprefpoint pdfsnapy pdfsnapycomp', PAGE + r"""
\T{snap}
\pg{\pdfsnaprefpoint x}
\pg{\pdfsnapy 10pt plus 2pt minus 2pt x}
\pg{\pdfsnapy 10pt x}
\pg{\pdfsnapy 0pt x}
\pg{\pdfsnapycomp 500 x}
\pg{\pdfsnapycomp 0 \pdfsnapycomp 1000 \pdfsnapycomp 1001 \pdfsnapycomp -1 x}
\shipout\vbox{\pdfsnaprefpoint\hbox{a}\pdfsnapy 20pt\hbox{b}\pdfsnapycomp 500 \pdfsnapy 13pt plus 3pt minus 3pt\hbox{c}}
\setbox1=\hbox{\pdfsnaprefpoint\pdfsnapy 1pt\pdfsnapycomp 5 }\showbox1
\setbox1=\vbox{\pdfsnaprefpoint\pdfsnapy 1pt\pdfsnapycomp 5 }\showbox1
\pdfsnaprefpoint\pdfsnapy 1pt\pdfsnapycomp 5
\hbox{}\vfill\penalty-10000
\pdfsnapy x\par
\pdfsnapycomp x\par
\show\pdfsnaprefpoint\show\pdfsnapy\show\pdfsnapycomp
""")
P('pdfinfo pdfcatalog pdfnames pdftrailer pdftrailerid', PAGE + r"""
\T{document dictionaries}
\pdfinfo{/Title (probe)}\pdfinfo{/Author (a) /Custom (\the\count0)}\pdfinfo{}
\pdfcatalog{/PageMode /UseNone}\pdfcatalog{/PageLayout /OneColumn}
\pdfcatalog{} openaction goto page 1 {/Fit}
\pdfcatalog{} openaction goto name {x}
\pdfcatalog{} openaction user {/S /URI /URI (u)}
\pdfnames{/Foo 1 0 R}\pdfnames{}
\pdftrailer{/Foo (bar)}\pdftrailer{}
\pdftrailerid{}\pdftrailerid{abc}
\immediate\pdfinfo{/Immediate (x)}
\global\pdfcatalog{/Global (x)}
\setbox1=\hbox{\pdfinfo{/InBox (x)}\pdfcatalog{/InBox (y)}\pdfnames{/InBox (z)}\pdftrailer{/InBox (w)}\pdftrailerid{}}\showbox1
\pdfinfo x\par
\pdfcatalog x\par
\pdfcatalog{} openaction x\par
\pdfnames x\par
\pdftrailer x\par
\pdftrailerid x\par
\pg{x}
\show\pdfinfo\show\pdfcatalog\show\pdfnames\show\pdftrailer\show\pdftrailerid
""")
P('pdfpageref', PAGE + r"""
\T{pdfpageref}
\V{\pdfpageref 1}
\pg{a}\pg{b}\V{\pdfpageref 1}\V{\pdfpageref 2}\V{\pdfpageref 3}\V{\pdfpageref 100}\V{\pdfpageref 0}\V{\pdfpageref -1}
\pdfpageref 1\par
\V{\pdfpageref x}\par
\edef\a{\pdfpageref 1}\show\a
\show\pdfpageref
\pg{\pdfpageref 2 }
""")
P('pdfinsertht', r"""
\T{pdfinsertht}
\V{\pdfinsertht100}
\V{\pdfinsertht254}
\V{\pdfinsertht255}
\V{\pdfinsertht0}
\V{\pdfinsertht x}
\V{\pdfinsertht-1}
\V{\pdfinsertht256}
\def\o{\V{in output \pdfinsertht100}\setbox0=\box255 \setbox0=\box100 \deadcycles=0 }\output={\o}
\hbox{a}\insert100{\hbox{note}}\V{\pdfinsertht100}\hbox{b}\insert100{\hbox{more}}\V{\pdfinsertht100}\penalty-10000
\V{\pdfinsertht100}
\edef\a{\pdfinsertht100}\show\a
\pdfinsertht100\par
\show\pdfinsertht
""")
P('pdfretval', r"""
\T{pdfretval}
\showthe\pdfretval
\pdfretval=1
\pdfretval
\V{\the\pdfretval}
\V{\pdfmatch{(}{a}}\V{\the\pdfretval}
\V{\pdfmatch{a}{a}}\V{\the\pdfretval}
\show\pdfretval
""")
# ---------------------------------------------------------------------------------------------- PDF font machinery
P('pdffontname pdffontobjnum pdffontsize', r"""
\T{pdf font info}
\V{\pdffontname\tenrm}\V{\pdffontobjnum\tenrm}\V{\pdffontsize\tenrm}
\font\f=cmr10 at 12pt \V{\pdffontname\f}\V{\pdffontobjnum\f}\V{\pdffontsize\f}
\font\g=cmbx10 \V{\pdffontname\g}\V{\pdffontobjnum\g}\V{\pdffontsize\g}
\V{\pdffontname\nullfont}\V{\pdffontobjnum\nullfont}\V{\pdffontsize\nullfont}
\V{\pdffontname\font}\V{\pdffontobjnum\font}\V{\pdffontsize\font}
\shipout\hbox{\f x\g y\tenrm z}
\V{\pdffontname\f}\V{\pdffontobjnum\f}\V{\pdffontsize\f}
\V{\pdffontname x}\par
\V{\pdffontobjnum x}\par
\V{\pdffontsize x}\par
\V{\pdffontname\relax}\par
\pdffontname\tenrm\par
\pdffontobjnum\tenrm\par
\pdffontsize\tenrm\par
\edef\a{\pdffontname\tenrm}\show\a
\show\pdffontname\show\pdffontobjnum\show\pdffontsize
""")
P('pdfincludechars', r"""
\T{pdfincludechars}
\pdfincludechars\tenrm{abc}\pdfincludechars\tenrm{}\pdfincludechars\tenbf{^^ff^^00}
\font\f=cmti10 \pdfincludechars\f{xyz}\pdfincludechars\nullfont{a}
\setbox1=\hbox{\pdfincludechars\tenrm{d}}\showbox1
\shipout\hbox{\tenrm x}
\pdfincludechars\tenrm{later}
\pdfincludechars x{a}\par
\pdfincludechars\tenrm x\par
\pdfincludechars\relax{a}\par
\font\h=cmr7 \pdfincludechars\h{only included}
\show\pdfincludechars
\immediate\pdfincludechars\tenrm{imm}
\global\pdfincludechars\tenrm{glo}
""")
P('pdffontattr', r"""
\T{pdffontattr}
\pdffontattr\tenrm{/Foo 1}\pdffontattr\tenrm{/Bar 2}\pdffontattr\tenbf{}\pdffontattr\nullfont{/X 1}
\font\f=cmti10 \pdffontattr\f{/Baz (\the\count0)}
\shipout\hbox{\tenrm x\tenbf y\f z}
\pdffontattr\tenrm{/Late 3}
\setbox1=\hbox{\pdffontattr\tenrm{/InBox 1}}\showbox1
\pdffontattr x{a}\par
\pdffontattr\tenrm x\par
\pdffontattr\relax{a}\par
\show\pdffontattr
\immediate\pdffontattr\tenrm{/Imm 1}
\global\pdffontattr\tenrm{/Glob 1}
""")
P('pdfmapfile pdfmapline', r"""
\T{map files}
\pdfmapline{cmr10 CMR10 <cmr10.pfb}
\pdfmapline{+cmr10 CMR10 <cmr10.pfb}
\pdfmapline{=cmr10 CMR10 <cmr10.pfb}
\pdfmapline{-cmr10}
\pdfmapline{=cmr10 CMR10 <cmr10.pfb}
\pdfmapline{}
\pdfmapline{\pc comment line}
\pdfmapline{*starred}
\pdfmapline{#hash}
\pdfmapline{;semi}
\pdfmapline{=}
\pdfmapline{+}
\pdfmapline{-}
\pdfmapline{=nofile NoFile}
\pdfmapline{=noformat}
\pdfmapline{=badops Foo "1 SlantFont 2 ExtendFont" <cmr10.pfb}
\pdfmapline{=unterminated Foo "1 SlantFont <cmr10.pfb}
\pdfmapline{=badslant Foo "9 SlantFont" <cmr10.pfb}
\pdfmapline{=badextend Foo "9 ExtendFont" <cmr10.pfb}
\pdfmapline{=badenc Foo <a.enc <b.enc <cmr10.pfb}
\pdfmapline{=badenc2 Foo "TeXBase1Encoding ReEncodeFont" <8r.enc <cmr10.pfb}
\pdfmapline{=flags Foo 4 <cmr10.pfb}
\pdfmapline{=flags2 Foo -1 <cmr10.pfb}
\pdfmapline{=flags3 Foo 2147483648 <cmr10.pfb}
\pdfmapline{=sub <<cmr10.pfb}
\pdfmapline{=sub2 <[cmr10.pfb}
\pdfmapline{=full <<cmr10.pfb}
\pdfmapline{=trail Foo <cmr10.pfb trailing junk}
\pdfmapline{=ttf Foo <cmr10.ttf}
\pdfmapline{=sfd@foo@ Foo <cmr10.pfb}
\pdfmapline{=sfd2@foo Foo <cmr10.pfb}
\pdfmapline{=name Foo <cmr10.pfb}
\pdfmapline{=name Bar <cmbx10.pfb}
\pdfmapfile{}
\pdfmapfile{nosuchmapfile.map}
\pdfmapfile{+nosuchmapfile.map}
\pdfmapfile{=nosuchmapfile.map}
\pdfmapfile{-nosuchmapfile.map}
\immediate\openout1=probe.map
\immediate\write1{\pc comment}
\immediate\write1{probeA ProbeA <cmr10.pfb}
\immediate\write1{probeB ProbeB <cmbx10.pfb}
\immediate\write1{}
\immediate\write1{=probeB ProbeBB <cmti10.pfb}
\immediate\closeout1
\pdfmapfile{+probe.map}
\pdfmapfile{=probe.map}
\pdfmapfile{-probe.map}
\pdfmapfile{probe.map}
\pdfsuppresswarningdupmap=1 \pdfmapline{+cmbx10 CMBX10 <cmbx10.pfb}\pdfmapfile{+probe.map}\pdfsuppresswarningdupmap=0
\setbox1=\hbox{\pdfmapline{=cmti10 CMTI10 <cmti10.pfb}\pdfmapfile{+probe.map}}\showbox1
\pdfmapline x\par
\pdfmapfile x\par
\pdfmapline{}}
\show\pdfmapfile\show\pdfmapline
\shipout\hbox{\tenrm a\tenbf b\tenit c}
\pdfmapline{=late Late <cmr10.pfb}
\pdfmapfile{late.map}
\shipout\hbox{\tenrm d}
\immediate\pdfmapline{=imm Imm <cmr10.pfb}
\global\pdfmapline{=glob Glob <cmr10.pfb}
""")
P('pdfgentounicode pdfglyphtounicode pdfnobuiltintounicode', r"""
\T{tounicode}
\showthe\pdfgentounicode
\pdfglyphtounicode{a}{0061}\pdfglyphtounicode{b}{0062 0063}\pdfglyphtounicode{c}{}\pdfglyphtounicode{}{0064}
\pdfglyphtounicode{tfm:cmr10/a}{0041}\pdfglyphtounicode{tfm:cmbx10/b}{0042}\pdfglyphtounicode{d}{zz}\pdfglyphtounicode{e}{00E9}
\pdfglyphtounicode{f}{1234 5678 9ABC}\pdfglyphtounicode{a}{0071}\pdfglyphtounicode{uni0041}{0041}\pdfglyphtounicode{u1F600}{D83D DE00}
\pdfgentounicode=1 \showthe\pdfgentounicode
\font\f=cmr10 at 11pt \pdfnobuiltintounicode\f
\shipout\hbox{\tenrm abcdef\tenbf abc\f abc}
\pdfgentounicode=0 \font\g=cmti10 at 11pt \shipout\hbox{\g abc}
\pdfgentounicode=-1 \showthe\pdfgentounicode \pdfgentounicode=2147483647 \pdfgentounicode=1
\pdfnobuiltintounicode\nullfont
\pdfnobuiltintounicode x\par
\pdfnobuiltintounicode\relax\par
\setbox1=\hbox{\pdfglyphtounicode{x}{0078}\pdfnobuiltintounicode\tenrm}\showbox1
\pdfglyphtounicode x\par
\pdfglyphtounicode{a}x\par
\pdfgentounicode=x
\show\pdfgentounicode\show\pdfglyphtounicode\show\pdfnobuiltintounicode
\immediate\pdfglyphtounicode{i}{0069}
\global\pdfglyphtounicode{g}{0067}
""")
P('pdfinterwordspaceon pdfinterwordspaceoff pdffakespace pdfspacefont', r"""
\T{interword spaces}
\pdfspacefont{pdftexspace}
\shipout\hbox{a\pdfinterwordspaceon b c\pdfinterwordspaceoff d\pdffakespace e}
\shipout\vbox{\hsize=100pt \noindent\pdfinterwordspaceon one two three four five six seven eight nine ten\pdfinterwordspaceoff\par}
\shipout\hbox{\pdffakespace\pdffakespace}
\pdfspacefont{cmr10}
\shipout\hbox{\pdfinterwordspaceon a b\pdffakespace\pdfinterwordspaceoff}
\pdfspacefont{}
\shipout\hbox{\pdfinterwordspaceon a b\pdfinterwordspaceoff}
\pdfspacefont{nosuchfontname}
\shipout\hbox{\pdfinterwordspaceon a b\pdfinterwordspaceoff}
\setbox1=\hbox{\pdfinterwordspaceon\pdfinterwordspaceoff\pdffakespace\pdfspacefont{cmr10}}\showbox1
\setbox1=\vbox{\pdfinterwordspaceon\pdfinterwordspaceoff\pdffakespace}\showbox1
\setbox1=\hbox{$\pdfinterwordspaceon\pdfinterwordspaceoff\pdffakespace$}\showbox1
\pdfinterwordspaceon\pdfinterwordspaceoff\pdffakespace
\hbox{}\vfill\penalty-10000
\pdfspacefont x\par
\pdfspacefont\relax{cmr10}\par
\show\pdfinterwordspaceon\show\pdfinterwordspaceoff\show\pdffakespace\show\pdfspacefont
""")
P('pdfcopyfont', r"""
\T{pdfcopyfont}
\pdfcopyfont\a=\tenrm \show\a\V{\fontname\a}\ifx\a\tenrm\V{same}\else\V{different}\fi
\pdfcopyfont\b\tenrm \show\b
\setbox1=\hbox{\a x\b y\tenrm z}\showbox1\shipout\copy1
\fontdimen2\a=1pt \showthe\fontdimen2\a\showthe\fontdimen2\tenrm
\hyphenchar\a=5 \showthe\hyphenchar\a\showthe\hyphenchar\tenrm
\lpcode\a`a=5 \showthe\lpcode\a`a\showthe\lpcode\tenrm`a
\pdfcopyfont\c=\nullfont \show\c
\pdfcopyfont\d=\font \show\d
\begingroup\pdfcopyfont\e=\tenrm \show\e\endgroup\show\e
\global\pdfcopyfont\f=\tenrm \show\f
\begingroup\global\pdfcopyfont\g=\tenrm \endgroup\show\g
\pdfcopyfont a=\tenrm
\pdfcopyfont\h=x\par
\pdfcopyfont\h=\relax\par
\pdfcopyfont\relax=\tenrm \show\relax\let\relax=\undefinedthing
\show\pdfcopyfont
""")
P('letterspacefont', r"""
\T{letterspacefont}
\letterspacefont\a=\tenrm 100 \show\a\V{\fontname\a}\ifx\a\tenrm\V{same}\else\V{different}\fi
\letterspacefont\b\tenrm -100 \show\b
\letterspacefont\c=\tenrm 0 \show\c
\letterspacefont\d=\tenrm 1000 \letterspacefont\e=\tenrm 1000 \ifx\d\e\V{shared}\else\V{not shared}\fi
\setbox1=\hbox{\a office\b office\c office\tenrm office}\showbox1\shipout\copy1
\V{\the\fontcharwd\a65 \the\fontcharht\a65 \the\fontchardp\a65 \the\fontcharic\a65}
\letterspacefont\f=\nullfont 100
\letterspacefont\g=\tenrm 2147483647
\letterspacefont\h=\tenrm -2147483647
\letterspacefont\i=\tenrm x\par
\letterspacefont\j=x\par
\letterspacefont a=\tenrm 1
\begingroup\letterspacefont\k=\tenrm 50 \show\k\endgroup\show\k
\global\letterspacefont\l=\tenrm 50 \show\l
\letterspacefont\m=\a 100 \show\m\setbox1=\hbox{\m x}\shipout\box1
\show\letterspacefont
""")
P('lpcode rpcode efcode tagcode knbscode stbscode shbscode knbccode knaccode', r"""
\T{character codes on fonts}
\def\codes#1{\showthe#1\tenrm`a #1\tenrm`a=100 \showthe#1\tenrm`a #1\tenrm`a=-100 \showthe#1\tenrm`a
  #1\tenrm`a=2147483647 \showthe#1\tenrm`a #1\tenrm`a=-2147483647 \showthe#1\tenrm`a
  #1\tenrm 0=5 \showthe#1\tenrm 0 #1\tenrm 255=5 \showthe#1\tenrm 255
  #1\tenrm 256=5
  #1\tenrm -1=5
  #1\tenrm x=5
  #1 x`a=5
  #1\relax`a=5
  #1\nullfont`a=5 \showthe#1\nullfont`a
  \begingroup#1\tenrm`b=7 \endgroup\showthe#1\tenrm`b
  \begingroup\global#1\tenrm`c=7 \endgroup\showthe#1\tenrm`c
  \advance#1\tenrm`d by 1 \showthe#1\tenrm`d \multiply#1\tenrm`d by 3 \showthe#1\tenrm`d \divide#1\tenrm`d by 2 \showthe#1\tenrm`d
  \V{\the#1\tenrm`d}\count1=#1\tenrm`d \showthe\count1 \show#1
  #1\tenrm`a=0 \noindent\par}
\codes\lpcode
\codes\rpcode
\codes\efcode
\codes\tagcode
\codes\knbscode
\codes\stbscode
\codes\shbscode
\codes\knbccode
\codes\knaccode
\pdfadjustinterwordglue=1 \pdfprependkern=1 \pdfappendkern=1 \pdfprotrudechars=2 \pdfadjustspacing=2
\font\m=cmr10 at 10pt \pdffontexpand\m 20 20 5 autoexpand
\lpcode\m`A=100 \rpcode\m`.=500 \efcode\m`e=500 \knbscode\m`,=500 \stbscode\m`,=100 \shbscode\m`,=100 \knbccode\m`a=200 \knaccode\m`b=200 \tagcode\m`a=1
\setbox1=\vbox{\hsize=100pt \tolerance=9999 \m A paragraph, with ab ab ab code effects, and more words, until a line must break. A second sentence. End.\par}\showbox1\shipout\copy1
\lpcode\m`A=1000 \lpcode\m`A=-1000 \rpcode\m`.=1000 \rpcode\m`.=-1000 \efcode\m`e=1000 \efcode\m`e=0
\setbox1=\vbox{\hsize=100pt \tolerance=9999 \m A paragraph, with ab ab ab code effects, and more words, until a line must break. A second sentence. End.\par}\showbox1\shipout\copy1
""")
P('pdfnoligatures', r"""
\T{pdfnoligatures}
\setbox1=\hbox{office}\showbox1
\font\f=cmr10 at 9pt \setbox1=\hbox{\f office fi fl}\showbox1
\pdfnoligatures\f \setbox1=\hbox{\f office fi fl}\showbox1\shipout\copy1
\pdfnoligatures\f \pdfnoligatures\nullfont
\pdfnoligatures x\par
\pdfnoligatures\relax\par
\show\pdfnoligatures
\pdfnoligatures\f=1
\setbox1=\hbox{\f\pdfnoligatures\font ffi}\showbox1
""")
P('pdffontexpand', r"""
\T{pdffontexpand}
\font\f=cmr10 at 9pt
\pdffontexpand\f 20 20 5 autoexpand \relax
\pdffontexpand\f 20 20 5 autoexpand \relax
\pdfadjustspacing=2
\setbox1=\vbox{\hsize=100pt \tolerance=9999 \f A paragraph with enough words to make several lines, so that the expansion is used in at least some of them.\par}\showbox1\shipout\copy1
\font\g=cmr10 at 8pt \pdffontexpand\g 0 0 0 autoexpand \relax
\setbox1=\vbox{\hsize=100pt \g one two three four five six seven eight nine ten\par}\showbox1\shipout\copy1
\font\g=cmr10 at 7pt \pdffontexpand\g 1000 1000 1000 autoexpand \relax
\setbox1=\vbox{\hsize=100pt \g one two three four five six seven eight nine ten\par}\showbox1\shipout\copy1
\font\g=cmr10 at 6pt \pdffontexpand\g 1001 1001 1 autoexpand \relax
\font\g=cmr10 at 5pt \pdffontexpand\g -1 -1 1 autoexpand \relax
\font\g=cmbx10 at 7pt \pdffontexpand\g 20 20 3 autoexpand \relax
\setbox1=\vbox{\hsize=100pt \g one two three four five six seven eight nine ten\par}\showbox1\shipout\copy1
\font\g=cmti10 at 7pt \pdffontexpand\g 20 10 5 autoexpand \relax
\setbox1=\vbox{\hsize=100pt \g one two three four five six seven eight nine ten\par}\showbox1\shipout\copy1
\font\g=cmti10 at 6pt \pdffontexpand\g 10 20 5 autoexpand \relax
\setbox1=\vbox{\hsize=100pt \g one two three four five six seven eight nine ten\par}\showbox1\shipout\copy1
\pdffontexpand\nullfont 20 20 5 autoexpand \relax
\pdffontexpand x 20 20 5 \relax
\pdffontexpand\g x 20 5 \relax
\pdffontexpand\g 20 x 5 \relax
\pdffontexpand\g 20 20 x \relax
\pdfadjustspacing=0
\setbox1=\hbox{\pdffontexpand\tenbf 20 20 5 autoexpand \relax}\showbox1
\show\pdffontexpand
\immediate\pdffontexpand\tenit 20 20 5 autoexpand \relax
\global\pdffontexpand\tensl 20 20 5 autoexpand \relax
""")
P('leftmarginkern rightmarginkern pdfprotrudechars pdfadjustspacing', r"""
\T{margin kerning}
\font\m=cmr10 at 10pt
\rpcode\m`.=700 \rpcode\m`,=500 \rpcode\m`-=700 \lpcode\m`A=100 \lpcode\m`T=100
\def\t{A paragraph with several words, so that it breaks, and then. The lines end with punctuation, or a word. Then more, and more text, until it ends. The end.}
\def\try#1#2{\pdfprotrudechars=#1 \pdfadjustspacing=#2 \setbox1=\vbox{\hsize=120pt \tolerance=9999 \m\t\par}\showbox1\shipout\copy1
  \setbox2=\vbox{\unvcopy1 \setbox3=\lastbox\ifhbox3 \V{[\leftmarginkern3 ][\rightmarginkern3 ]}\fi\unskip\unpenalty\setbox3=\lastbox\ifhbox3 \V{[\leftmarginkern3 ][\rightmarginkern3 ]}\fi}
  \V{\the\prevgraf}}
\try00
\try10
\try20
\try30
\try01
\try02
\try03
\try11
\try22
\try{-1}{-1}
\pdffontexpand\m 20 20 5 autoexpand \relax
\try01
\try02
\try22
\pdfprotrudechars=0 \pdfadjustspacing=0
\setbox1=\hbox{\m .}\V{[\leftmarginkern1 ][\rightmarginkern1 ]}
\setbox1=\vbox{}\V{[\leftmarginkern1 ][\rightmarginkern1 ]}
\setbox1=\box2 \V{[\leftmarginkern1 ][\rightmarginkern1 ]}
\V{\leftmarginkern x}\par
\V{\rightmarginkern x}\par
\V{\leftmarginkern 32768}\par
\V{\rightmarginkern -1}\par
\leftmarginkern 1\par
\rightmarginkern 1\par
\edef\a{\leftmarginkern 1 \rightmarginkern 1 }\show\a
\show\leftmarginkern\show\rightmarginkern
\setbox1=\vbox{\pdfprotrudechars=2 \hsize=100pt \tolerance=9999 \m\leftskip=5pt \rightskip=5pt \t\par}\showbox1\shipout\copy1
\setbox1=\vbox{\pdfprotrudechars=2 \hsize=100pt \tolerance=9999 \m\pdfeachlineheight=20pt \t\par}\showbox1\shipout\copy1
""")
P('pdfadjustinterwordglue pdfprependkern pdfappendkern', r"""
\T{interword glue and kerns from codes}
\font\m=cmr10 at 10pt
\knbscode\m`,=500 \stbscode\m`,=200 \shbscode\m`,=100 \knbscode\m`a=100 \knbccode\m`b=300 \knaccode\m`c=300
\def\t{word, word ab bc ca, word.}
\def\try{\setbox1=\hbox{\m\t}\showbox1\shipout\copy1 \setbox1=\vbox{\hsize=60pt \tolerance=9999 \m\t\t\t\par}\showbox1\shipout\copy1 }
\try\pdfadjustinterwordglue=1 \try\pdfadjustinterwordglue=2 \try\pdfadjustinterwordglue=0
\pdfprependkern=1 \try\pdfprependkern=0
\pdfappendkern=1 \try\pdfappendkern=0
\pdfadjustinterwordglue=1 \pdfprependkern=1 \pdfappendkern=1 \try
\setbox1=\hbox{\m f\/a\noboundary b\accent127 c\char`d\discretionary{a}{b}{c}}\showbox1
\pdfadjustinterwordglue=-1 \pdfprependkern=-1 \pdfappendkern=-1 \try
\pdfadjustinterwordglue=0 \pdfprependkern=0 \pdfappendkern=0
""")
P('pdfpageheight pdfpagewidth pdfhorigin pdfvorigin', r"""
\T{page geometry}
\showthe\pdfpageheight\showthe\pdfpagewidth\showthe\pdfhorigin\showthe\pdfvorigin
\shipout\hbox{default}
\pdfpageheight=100pt \pdfpagewidth=200pt \shipout\hbox{small}
\pdfpageheight=0pt \pdfpagewidth=0pt \shipout\vbox{\hbox{from box}\hbox{size}}
\pdfhorigin=0pt \pdfvorigin=0pt \shipout\vbox{\hbox{no origin}}
\pdfhorigin=5in \pdfvorigin=5in \shipout\vbox{\hbox{large origin}}
\pdfhorigin=-1in \pdfvorigin=-1in \shipout\vbox{\hbox{negative origin}}
\pdfhorigin=1in \pdfvorigin=1in \pdfpageheight=16383pt \pdfpagewidth=16383pt \shipout\vbox{\hbox{max}}
\pdfpageheight=-1pt \pdfpagewidth=-1pt \shipout\vbox{\hbox{negative}}
\pdfpageheight=0.3pt \pdfpagewidth=0.3pt \shipout\vbox{\hbox{tiny}}
\pdfpageheight=841.89pt \pdfpagewidth=595.27pt \hoffset=1pt \voffset=2pt \shipout\vbox{\hbox{offsets}}
\hoffset=0pt \voffset=0pt
\pdfdecimaldigits=0 \pdfpageheight=841.89pt \shipout\hbox{d0}\pdfdecimaldigits=2 \shipout\hbox{d2}\pdfdecimaldigits=3 \shipout\hbox{d3}\pdfdecimaldigits=4 \shipout\hbox{d4}\pdfdecimaldigits=3
""")
P('pdfpageattr pdfpagesattr pdfpageresources', r"""
\T{page attributes}
\showthe\pdfpageattr\showthe\pdfpagesattr\showthe\pdfpageresources
\pdfpageattr={/Rotate 90}\shipout\hbox{rotated}\pdfpageattr={}
\pdfpageattr={/CropBox [0 0 100 100]}\shipout\hbox{crop}\pdfpageattr={}
\pdfpagesattr={/Rotate 180}\shipout\hbox{pages attr}
\pdfpageresources={/ExtGState << /G << /CA 0.5 >> >>}\shipout\hbox{\pdfliteral{/G gs}res}\pdfpageresources={}
\pdfpageattr={\the\count0}\shipout\hbox{tokens}\pdfpageattr={}
\begingroup\pdfpageattr={/Local 1}\shipout\hbox{local}\endgroup\shipout\hbox{not local}
\toks0={/FromToks 1}\pdfpageattr=\toks0 \shipout\hbox{from toks}\pdfpageattr={}
\pdfpageattr={\undefinedthing}\shipout\hbox{undefined in attr}\pdfpageattr={}
\pdfpagesattr={/Late 1}
\pdfpageattr=x\par
\pdfpagesattr=x\par
\pdfpageresources=x\par
""")
P('pdfdraftmode pdfomitcharset pdfomitinfodict pdfomitprocset pdfsuppressptexinfo pdfinfoomitdate pdfptexuseunderscore pdfuniqueresname pdftracingfonts pdfmajorversion pdfminorversion pdfoptionpdfminorversion pdfcompresslevel pdfobjcompresslevel pdfdecimaldigits pdfpkresolution pdfpkmode pdfpxdimen pdfmovechars pdfimageresolution pdfimagehicolor pdfimagegamma pdfimageapplygamma pdfgamma pdfpagebox pdfforcepagebox pdfoptionalwaysusepdfpagebox pdfoptionpdfinclusionerrorlevel pdfinclusionerrorlevel pdfinclusioncopyfonts pdfsuppresswarningdupdest pdfsuppresswarningdupmap pdfsuppresswarningpagegroup', r"""
\T{output parameter effect}
\shipout\hbox{default \tenbf setting}
\def\obj{\immediate\pdfobj {<<>>}}
\obj\pdfinfo{/Title (x)}
\shipout\vbox{\hbox{\tenit second}\hrule width 3.3333pt height .7pt}
""")
P('pdffirstlineheight pdflastlinedepth pdfeachlineheight pdfeachlinedepth pdfignoreddimen', r"""
\T{line height parameters}
\def\t{one two three four five six seven eight nine ten eleven twelve thirteen fourteen}
\hsize=60pt
\def\try{\setbox1=\vbox{\noindent\t\par}\showbox1 \setbox1=\vbox{\noindent\t$$a$$\t\par}\showbox1 }
\showthe\pdfignoreddimen\showthe\pdffirstlineheight\showthe\pdflastlinedepth\showthe\pdfeachlineheight\showthe\pdfeachlinedepth
\try
\pdffirstlineheight=20pt \try\pdffirstlineheight=\pdfignoreddimen
\pdflastlinedepth=20pt \try\pdflastlinedepth=\pdfignoreddimen
\pdfeachlineheight=20pt \try\pdfeachlineheight=\pdfignoreddimen
\pdfeachlinedepth=20pt \try\pdfeachlinedepth=\pdfignoreddimen
\pdffirstlineheight=20pt \pdflastlinedepth=20pt \pdfeachlineheight=20pt \pdfeachlinedepth=20pt \try
\pdfignoreddimen=0pt \try \pdfignoreddimen=-1000pt
\pdffirstlineheight=0pt \pdflastlinedepth=0pt \pdfeachlineheight=0pt \pdfeachlinedepth=0pt \try
\pdffirstlineheight=-1000pt \pdflastlinedepth=-1000pt \pdfeachlineheight=-1000pt \pdfeachlinedepth=-1000pt \hsize=200pt
""")

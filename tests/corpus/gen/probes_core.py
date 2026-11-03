r"""Hand-written probes for the TeX82 primitives: macros, expansion, conditionals, registers, boxes, math, alignment, I/O.

SPECIFIC maps a primitive name to TeX that really uses it, success paths and error paths both.  \T{..} and \V{..}, defined by
primitives.py, write a line to the log only.  A probe may leave TeX in an odd state after an error, and that is the point:
recovery has to match too.  Anything that ends the job goes in FATAL in primitives.py instead, so that it cannot hide the rest.
"""
SPECIFIC = {}


def P(names, body):
    for n in names.split() or [names]:      # the control-space primitive has the name ' '
        SPECIFIC[n] = SPECIFIC.get(n, '') + body


# ---------------------------------------------------------------------------------------------- macro definitions
P('def', r"""
\T{def forms}
\def\a{x}\show\a
\def\a#1{[#1]}\show\a \V{\a{y}}
\def\a#1#2#3#4#5#6#7#8#9{#9#8#7#6#5#4#3#2#1}\V{\a123456789}
\def\a#1.#2/{#2#1}\show\a \V{\a x.y/}
\def\a#1#{(#1)}\show\a \V{\a xyz{}}
\def\a x#1y#2z{#1,#2}\V{\a x12y34z}
\def\a#1{#1#1}\V{\a{ab}}
\def\a{##}\show\a
\def\a#1{\def\b##1{#1##1}}\a{p}\show\b
\T{def errors}
\def\a#2{x}
\def\a#1#3{x}
\def\a#1#2#3#4#5#6#7#8#9#10{x}
\def\a{#1}
\def\a#1{#2}
\def\a#1{#}
\def\a#1x#1{y}
\def a{b}
\def{b}
\def\a}
\def\a#1{x}\a}
\def\a#1.{x}\V{\a y}.
\def\a#1.{\relax}\a{.}.
\def\a#1\b{\relax}\a x\par \b
\long\def\b#1\c{\relax}\b x\par \c
\def\a#1{\relax}\a\par
\show\a
\def\a{\b}\def\b{\c}\def\c{\relax}\a
""")
P('gdef', r"""
\T{gdef}
\begingroup\gdef\a{global}\endgroup\show\a
\begingroup\gdef\a#1{g#1}\show\a\endgroup\V{\a1}
\begingroup\global\def\a{x}\endgroup\show\a
\begingroup\long\gdef\a#1{#1}\endgroup\show\a
\begingroup\protected\gdef\a{y}\endgroup\show\a
\gdef a{b}
""")
P('edef', r"""
\T{edef}
\def\b{B}\edef\a{x\b y}\show\a
\edef\a{\noexpand\b}\show\a
\edef\a{\the\count0\space\number5\romannumeral9}\show\a
\edef\a{\string\b\meaning\b}\show\a
\edef\a{\ifnum1=1 yes\else no\fi}\show\a
\def\c#1{<#1>}\edef\a{\c{\b}}\show\a
\edef\a#1{#1\b}\show\a
\edef\a{##}\show\a
\toks0={\b#}\edef\a{\the\toks0}\show\a
\edef\a{\unexpanded{\b#}}\show\a
\edef\a{\relax\par\hbox}\show\a
\edef\a{\undefinedthing}\show\a
\protected\def\p{P}\edef\a{\p\b}\show\a
\edef\a{\expandafter\noexpand\b}\show\a
\edef\a{\a\a}\show\a
\edef\a{\csname\endcsname\csname b\endcsname}\show\a
""")
P('xdef', r"""
\T{xdef}
\def\b{B}\begingroup\xdef\a{x\b}\endgroup\show\a
\begingroup\def\b{C}\xdef\a{\b}\endgroup\show\a
\begingroup\long\xdef\a#1{#1\b}\endgroup\show\a
\xdef\a#1#3{}
""")
P('long', r"""
\T{long}
\long\def\a#1{[#1]}\show\a \a{x\par y}\par
\def\a#1{[#1]}\a{x\par y}\par
\long\long\def\a{x}\show\a
\long\outer\def\o{x}\show\o
\long\let\a=\relax
\long\relax
\long x\par
\long\global\def\a{y}\show\a
\long\protected\def\a{y}\show\a
""")
P('outer', r"""
\T{outer}
\outer\def\o{x}\show\o
\def\a{\o}
\def\a#1{#1}\a\o
\def\a#1{#1}\a{\o}
\edef\a{\o}
\begingroup\setbox0=\hbox{\o}\endgroup
\iffalse \o \fi
\message{\o}
\csname o\endcsname
\expandafter\show\csname o\endcsname
\outer\let\a=\relax
\outer\relax
\let\o=\relax
""")
P('global', r"""
\T{global}
\begingroup\global\count10=5 \endgroup\showthe\count10
\begingroup\global\dimen10=5pt \endgroup\showthe\dimen10
\begingroup\global\skip10=5pt \endgroup\showthe\skip10
\begingroup\global\muskip10=5mu \endgroup\showthe\muskip10
\begingroup\global\toks10={g}\endgroup\showthe\toks10
\begingroup\global\setbox10=\hbox{g}\endgroup\showbox10
\begingroup\global\let\a=\relax\endgroup\show\a
\begingroup\global\chardef\a=5 \endgroup\show\a
\begingroup\global\catcode`\Q=11 \endgroup\showthe\catcode`\Q
\begingroup\global\font\a=cmr10 \endgroup\show\a
\begingroup\global\hyphenchar\tenrm=45 \endgroup\showthe\hyphenchar\tenrm
\begingroup\global\wd10=1pt \endgroup\showbox10
\global\relax
\global x\par
\global\global\def\a{}
\begingroup\global\tolerance=123 \endgroup\showthe\tolerance
\begingroup\count11=1 \global\count11=2 \count11=3 \endgroup\showthe\count11
""")
P('let', r"""
\T{let}
\let\a=b\show\a
\let\a b\show\a
\let\a= b\show\a
\let\a=  =\show\a
\let\a=\relax\show\a
\let\a=\undefinedthing\show\a
\def\b{x}\let\a=\b\show\a\def\b{y}\show\a
\let\a= {\show\a\let\a= }\show\a
\let\a=^\show\a \let\a=#\show\a \let\a=$\show\a \let\a=&\show\a
\let\a= \space\show\a
\catcode`\~=13 \let~=\relax\show~
\let a=b
\let
\relax
\let\if=\iftrue
\show\if
\let\if=\iffalse
\let\a=\par\show\a
\let\b=\a\show\b
\ifx\a\par\V{same}\fi
\begingroup\let\relax=x\show\relax\endgroup\show\relax
""")
P('futurelet', r"""
\T{futurelet}
\def\b{\show\a}\futurelet\a\b x\par
\futurelet\a\b{}\par
\futurelet\a\b\relax
\futurelet\a\b\undefinedthing
\futurelet\a\b$\relax$
\futurelet\a\b  y\par
\futurelet a\b
\def\c{\ifx\a\d\V{next is d}\else\V{next is not}\fi}\def\d{D}\futurelet\a\c\d\par
\global\futurelet\a\relax\relax\show\a
""")
P('chardef', r"""
\T{chardef}
\chardef\a=65 \show\a\V{\the\a}\V{\meaning\a}\V{\number\a}
\chardef\a=255 \show\a
\chardef\a=256
\chardef\a=-1
\chardef\a="41 \setbox0=\hbox{\a}\showbox0
\count1=\a\showthe\count1
\chardef a=1
\chardef\relax=1 \show\relax \let\relax=\undefinedthing
""")
P('mathchardef', r"""
\T{mathchardef}
\mathchardef\a="7FFF \show\a\V{\the\a}\V{\meaning\a}
\mathchardef\a="8000
\mathchardef\a=-1
\mathchardef\a="0141 \setbox0=\hbox{$\a$}\showbox0
\setbox0=\hbox{\a}\showbox0
\mathchardef\a=2147483647
""")
P('countdef', r"""
\T{countdef}
\countdef\a=10 \a=5 \show\a\showthe\a\showthe\count10
\countdef\a=255 \a=6 \showthe\count255
\countdef\a=256 \a=7 \showthe\count256
\countdef\a=32767 \a=8 \showthe\count32767 \show\a
\countdef\a=32768
\countdef\a=-1
\countdef a=1
\V{\meaning\a}\advance\a by 1 \showthe\a
""")
P('dimendef', r"""
\T{dimendef}
\dimendef\a=10 \a=5pt \show\a\showthe\a\showthe\dimen10
\dimendef\a=32767 \a=8pt \showthe\dimen32767
\dimendef\a=32768
\dimendef\a=-1
\advance\a by 1pt \showthe\a\V{\meaning\a}
""")
P('skipdef', r"""
\T{skipdef}
\skipdef\a=10 \a=5pt plus 1fil \show\a\showthe\a\showthe\skip10
\skipdef\a=32767 \a=8pt \showthe\skip32767
\skipdef\a=32768
\advance\a by 1pt \showthe\a\V{\meaning\a}
""")
P('muskipdef', r"""
\T{muskipdef}
\muskipdef\a=10 \a=5mu plus 1fil \show\a\showthe\a\showthe\muskip10
\muskipdef\a=32767 \a=8mu \showthe\muskip32767
\muskipdef\a=32768
\a=5pt \advance\a by 1mu \showthe\a\V{\meaning\a}
""")
P('toksdef', r"""
\T{toksdef}
\toksdef\a=10 \a={x#} \show\a\showthe\a\showthe\toks10
\toksdef\a=32767 \a={y}\showthe\toks32767
\toksdef\a=32768
\a=\a\showthe\a\V{\meaning\a}
\a\expandafter{\the\a z}\showthe\a
""")
# ---------------------------------------------------------------------------------------------- expansion
P('expandafter', r"""
\T{expandafter}
\def\a{A}\def\b{B}
\edef\c{\expandafter\noexpand\a\b}\show\c
\expandafter\def\csname foo\endcsname{bar}\show\foo
\expandafter\show\csname foo\endcsname
\expandafter\expandafter\expandafter\show\expandafter\a\b
\expandafter\relax\relax
\expandafter\ifx\a\b\V{same}\else\V{differ}\fi
\expandafter\ifx\csname undefinedname\endcsname\relax\V{relax}\fi
\expandafter\expandafter\show\a
\expandafter\relax\fi
\def\d#1{[#1]}\V{\expandafter\d\expandafter{\a}}
\expandafter\par\par
\expandafter}
""")
P('noexpand', r"""
\T{noexpand}
\def\a{A}\edef\b{\noexpand\a\a}\show\b
\noexpand\a\par
\edef\b{\noexpand\relax\noexpand x\noexpand{}}\show\b
\edef\b{\noexpand}\show\b
\ifx\noexpand\a\a\V{same}\else\V{differ}\fi
\if\noexpand\a\relax\V{if-true}\else\V{if-false}\fi
\expandafter\show\noexpand\a
\edef\b{\noexpand\fi\noexpand\else}\show\b
""")
P('csname', r"""
\T{csname}
\csname relax\endcsname
\expandafter\show\csname a b\endcsname
\expandafter\show\csname\endcsname
\expandafter\show\csname \string\relax\endcsname
\def\n{xyz}\expandafter\show\csname\n\endcsname
\expandafter\show\csname\n\n\number5\endcsname
\expandafter\let\csname q\endcsname=\relax\show\q
\expandafter\def\csname q r\endcsname{1}\expandafter\show\csname q r\endcsname
\csname\relax\endcsname
\csname a\relax b\endcsname
\csname\par\endcsname
\csname\hbox\endcsname
\expandafter\show\csname par\endcsname
\expandafter\show\csname ^^80\endcsname
\expandafter\show\csname ^^ff^^ff\endcsname
\csname undefinedcs
\endcsname
""")
P('endcsname', r"""
\T{endcsname}
\endcsname
\expandafter\show\csname x\endcsname
\setbox0=\hbox{\endcsname}
\show\endcsname
\csname\endcsname
\let\e=\endcsname \expandafter\show\csname y\e
""")
P('string', r"""
\T{string}
\V{\string\a\string\relax\string a\string{\string}\string#\string$}
\V{\string\ \string\^^M\string\par\string~}
\escapechar=-1 \V{\string\a}\escapechar=`! \V{\string\a}\escapechar=`\\ 
\escapechar=256 \V{\string\a}\escapechar=`\\
\V{\string^^80\string^^ff}
\def\a{\string}\V{\a x}
\string\relax\par
\edef\b{\string\a}\show\b
\V{\string\csname\string\endcsname}
""")
P('meaning', r"""
\T{meaning}
\def\a#1.{x#1}\V{\meaning\a}\V{\meaning\relax}\V{\meaning a}\V{\meaning 1}\V{\meaning{}\V{\meaning}}
\V{\meaning\undefinedthing}\V{\meaning\par}\V{\meaning\tenrm}\V{\meaning\nullfont}
\V{\meaning ~}\V{\meaning #}\V{\meaning$}\V{\meaning^}\V{\meaning_}\V{\meaning&}\V{\meaning\ }
\long\outer\def\o{y}\V{\meaning\o}\protected\def\p{z}\V{\meaning\p}
\chardef\c=5 \V{\meaning\c}\mathchardef\c=5 \V{\meaning\c}\countdef\c=5 \V{\meaning\c}
\dimendef\c=5 \V{\meaning\c}\skipdef\c=5 \V{\meaning\c}\muskipdef\c=5 \V{\meaning\c}\toksdef\c=5 \V{\meaning\c}
\let\c= \space\V{\meaning\c}
\meaning\relax\par
""")
P('number', r"""
\T{number}
\V{\number5\number-5\number"FF\number'17\number`a}
\V{\number\count0\number\dimen0\number\skip0\number\baselineskip}
\V{\number2147483647}\V{\number2147483648}
\V{\number}\V{\number x}
\V{\number\number5}
\number5\par
""")
P('romannumeral', r"""
\T{romannumeral}
\V{\romannumeral1\romannumeral4\romannumeral9\romannumeral40\romannumeral90\romannumeral400}
\V{\romannumeral1999\romannumeral3888\romannumeral4000\romannumeral10000}
\V{\romannumeral0\romannumeral-1\romannumeral-10000}
\V{\romannumeral2147483647}
\expandafter\show\romannumeral-`0\relax
\def\a{x}\edef\b{\romannumeral-`0\a}\show\b
\V{\romannumeral}\romannumeral5\par
""")
P('the', r"""
\T{the}
\V{\the\count0\the\dimen0\the\skip0\the\muskip0\the\toks0}
\V{\the\catcode`a\the\lccode`a\the\uccode`a\the\sfcode`a\the\mathcode`a\the\delcode`a}
\V{\the\tenrm\the\font\the\textfont0\the\scriptfont0\the\scriptscriptfont0}
\V{\the\wd0\the\ht0\the\dp0}
\V{\the\parshape\the\hangafter\the\prevgraf\the\inputlineno}
\V{\the\fontdimen1\tenrm\the\hyphenchar\tenrm\the\skewchar\tenrm}
\chardef\c=5 \V{\the\c}\mathchardef\c=5 \V{\the\c}
\V{\the\numexpr1\relax\the\dimexpr1pt\relax\the\glueexpr1pt\relax\the\muexpr1mu\relax}
\V{\the\relax}\V{\the x}\V{\the\par}\V{\the\hbox}\V{\the}
\the\count0\par
\edef\a{\the\toks0}\show\a
\toks0={a#b}\edef\a{\the\toks0}\show\a
\V{\the\lastpenalty\the\lastkern\the\lastskip\the\badness}
""")
P('relax', r"""
\T{relax}
\relax
\setbox0=\hbox{\relax a\relax}\showbox0
\setbox0=\hbox{$\relax a\relax$}\showbox0
\count1=1\relax\showthe\count1
\ifx\relax\undefinedthing\V{a}\else\V{b}\fi
\expandafter\ifx\csname undefinedname\endcsname\relax\V{undefined is relax}\fi
\edef\a{\relax}\show\a
\begingroup\def\relax{x}\show\relax\endgroup\show\relax
\V{\meaning\relax}
\halign{#\relax\cr\relax a\cr}
\relax}
""")
P('begingroup', r"""
\T{begingroup}
\begingroup\count1=5 \showthe\currentgrouplevel\showthe\currentgrouptype\endgroup\showthe\count1
\begingroup\begingroup\showgroups\endgroup\endgroup
\begingroup}
\endgroup
\begingroup{\endgroup}
\setbox0=\hbox{\begingroup a\endgroup b}\showbox0
\setbox0=\hbox{\begingroup a}\showbox0\endgroup
\aftergroup\relax
""")
P('endgroup', r"""
\T{endgroup}
\endgroup
\begingroup\endgroup\endgroup
{\endgroup}
\setbox0=\hbox{\endgroup}
\begingroup{\endgroup}\endgroup
$\endgroup$
""")
P('aftergroup', r"""
\T{aftergroup}
\def\a{\V{a}}\def\b{\V{b}}
{\aftergroup\a\aftergroup\b\V{in}}\V{out}
\begingroup\aftergroup\a\endgroup
{{\aftergroup\a}\aftergroup\b}
\aftergroup\a
\setbox0=\hbox{\aftergroup\a x}\showbox0
\setbox0=\hbox{\aftergroup A\aftergroup B x}\showbox0
\setbox0=\hbox{\aftergroup$x}
\aftergroup}
{\aftergroup{\V{inner}\aftergroup}}
{\aftergroup\par}\par
{\aftergroup\fi}
""")
P('afterassignment', r"""
\T{afterassignment}
\def\a{\V{after}}
\afterassignment\a\count1=5
\afterassignment\a\def\b{x}\show\b
\afterassignment\a\afterassignment\relax\count1=6
\afterassignment\a\setbox0=\hbox{\V{inside}x}\showbox0
\afterassignment\a\relax
\afterassignment\a\let\b=c
\afterassignment\a\chardef\b=5
\afterassignment\a\font\b=cmr10
\afterassignment\a\global\count1=7
\afterassignment A\count1=8 \par
\afterassignment}
\afterassignment\undefinedthing\count1=9
\afterassignment\fi\count1=1
\afterassignment\a\tolerance=100
\afterassignment\a\toks0={x}
\afterassignment\a\everypar={}
\afterassignment\a\hsize=100pt
\afterassignment\a\lowercase{x}\par
""")
P('ignorespaces', r"""
\T{ignorespaces}
\setbox0=\hbox{a\ignorespaces   b}\showbox0
\setbox0=\hbox{a\ignorespaces \space\space b}\showbox0
\def\a{ }\setbox0=\hbox{a\ignorespaces\a\a b}\showbox0
\setbox0=\hbox{a\ignorespaces\relax b}\showbox0
\ignorespaces
\ignorespaces\relax
""")
P('uppercase lowercase', r"""
\T{change case}
\uppercase{\V{abc}}\lowercase{\V{ABC}}
\uppercase{\def\a{xy}}\show\a
\lowercase{\def\a{XY}}\show\a
\uccode`x=`y \uppercase{\def\a{xy}}\show\a \uccode`x=`X
\lccode`X=0 \lowercase{\def\a{XY}}\show\a \lccode`X=`x
\uppercase\expandafter{\V{\a}}
\def\m{abc}\uppercase\expandafter{\expandafter\def\expandafter\a\expandafter{\m}}\show\a
\uppercase x
\lowercase{\relax}\uppercase{\relax}
\uppercase{\show ~}\lowercase{\show ~}
\lowercase{\uppercase{\V{Mix}}}
\uppercase}
\lowercase{}
""")
# ---------------------------------------------------------------------------------------------- conditionals
CONDITION_TAIL = r"""
\T{conditional structure}
\unless\iftrue\V{unless-then}\else\V{unless-else}\fi
\showifs
\iftrue\showifs\iffalse\else\showifs\fi\fi
\iffalse\if\else\ifcat\or\fi\fi\fi\V{skipped nest}
\iftrue\else\if\fi\fi
\T{conditional errors}
\else
\or
\fi
\iftrue\or\fi
\iffalse\or\fi
\iftrue\else\else\fi
"""
P('if', r"""
\T{if}
\if aa\V{1t}\else\V{1f}\fi
\if ab\V{2t}\else\V{2f}\fi
\if a\relax\V{3t}\else\V{3f}\fi
\if\relax\par\V{4t}\else\V{4f}\fi
\def\a{xy}\if\a\V{5t}\else\V{5f}\fi
\def\e{}\if\e aa\V{6t}\else\V{6f}\fi
\if\noexpand\a\relax\V{7t}\else\V{7f}\fi
\if~~\V{8t}\else\V{8f}\fi
\catcode`\~=13 \def~{a}\if~a\V{9t}\else\V{9f}\fi\if\noexpand~\relax\V{10t}\else\V{10f}\fi
\catcode`\~=12 \if~\relax\V{11t}\else\V{11f}\fi
\if{}\V{12t}\else\V{12f}\fi
\if\bgroup\egroup\V{13t}\else\V{13f}\fi
\if 1\ifnum1=1 1\else2\fi\V{14t}\else\V{14f}\fi
\if\fi
\if a\fi
\if aa\if bb\else\fi\fi
\if\else\fi
""" + CONDITION_TAIL)
P('ifcat', r"""
\T{ifcat}
\ifcat ab\V{1t}\else\V{1f}\fi
\ifcat a1\V{2t}\else\V{2f}\fi
\ifcat 1=\V{3t}\else\V{3f}\fi
\ifcat\relax\par\V{4t}\else\V{4f}\fi
\ifcat a\relax\V{5t}\else\V{5f}\fi
\ifcat{}\V{6t}\else\V{6f}\fi
\ifcat\bgroup{\V{7t}\else\V{7f}\fi}
\ifcat$$\V{8t}\else\V{8f}\fi
\ifcat&&\V{9t}\else\V{9f}\fi
\ifcat##\V{10t}\else\V{10f}\fi
\ifcat^^\V{11t}\else\V{11f}\fi
\ifcat__\V{12t}\else\V{12f}\fi
\catcode`\~=13 \def~{a}\ifcat~a\V{13t}\else\V{13f}\fi\ifcat\noexpand~\relax\V{14t}\else\V{14f}\fi
\chardef\c=`a \ifcat\c\relax\V{15t}\else\V{15f}\fi
\ifcat\fi
""" + CONDITION_TAIL)
P('ifnum', r"""
\T{ifnum}
\ifnum1<2 \V{1t}\else\V{1f}\fi\ifnum2<1 \V{2t}\else\V{2f}\fi
\ifnum1=1 \V{3t}\else\V{3f}\fi\ifnum2>1 \V{4t}\else\V{4f}\fi
\ifnum2147483647>-2147483647 \V{5t}\fi
\ifnum\count0=\count0 \V{6t}\fi
\ifnum`a=97 \V{7t}\fi
\ifnum"FF='377 \V{8t}\fi
\ifnum1pt=65536 \V{9t}\fi
\ifnum\dimen0=0 \V{10t}\fi
\ifnum\baselineskip>0 \V{11t}\fi
\ifnum1 \V{no relation}\fi
\ifnum1x2 \V{bad relation}\fi
\ifnum x<1 \V{no number}\fi
\ifnum1<x \V{no number}\fi
\ifnum2147483648>0 \V{big}\fi
\ifnum1<2\fi
\ifnum\fi
\unless\ifnum1>2 \V{12t}\fi
""" + CONDITION_TAIL)
P('ifdim', r"""
\T{ifdim}
\ifdim1pt<2pt \V{1t}\else\V{1f}\fi\ifdim2pt<1pt \V{2t}\else\V{2f}\fi
\ifdim1pt=1pt \V{3t}\else\V{3f}\fi\ifdim1pt=65536sp \V{4t}\else\V{4f}\fi
\ifdim1in=72.27pt \V{5t}\else\V{5f}\fi\ifdim1in=2.54cm \V{6t}\else\V{6f}\fi
\ifdim1cm=10mm \V{7t}\else\V{7f}\fi\ifdim1bp=0.99626pt \V{8t}\else\V{8f}\fi
\ifdim1pc=12pt \V{9t}\else\V{9f}\fi\ifdim1dd=1.07pt \V{10t}\else\V{10f}\fi\ifdim1cc=12dd \V{11t}\else\V{11f}\fi
\ifdim\maxdimen>16383pt \V{12t}\fi
\ifdim\baselineskip>0pt \V{13t}\fi
\ifdim1\baselineskip=\baselineskip \V{14t}\fi
\ifdim 1pt\V{no relation}\fi
\ifdim x<1pt \V{no dim}\fi
\ifdim1pt<x \V{no dim}\fi
\ifdim16384pt>0pt \V{big}\fi
\ifdim1<2pt \V{no unit}\fi
\ifdim\fi
""" + CONDITION_TAIL)
P('ifodd', r"""
\T{ifodd}
\ifodd1 \V{1t}\else\V{1f}\fi\ifodd2 \V{2t}\else\V{2f}\fi
\ifodd-1 \V{3t}\else\V{3f}\fi\ifodd0 \V{4t}\else\V{4f}\fi
\ifodd2147483647 \V{5t}\else\V{5f}\fi
\ifodd\count0 \V{6t}\else\V{6f}\fi
\ifodd`a \V{7t}\else\V{7f}\fi
\ifodd x \V{8t}\else\V{8f}\fi
\ifodd\fi
""" + CONDITION_TAIL)
P('ifvmode ifhmode ifmmode ifinner', r"""
\T{mode conditionals}
\def\m{\ifvmode\V{v}\else\V{not v}\fi\ifhmode\V{h}\else\V{not h}\fi\ifmmode\V{m}\else\V{not m}\fi\ifinner\V{inner}\else\V{outer}\fi}
\m
\setbox0=\vbox{\m}
\setbox0=\hbox{\m$\m$}
$$\m\halign{#\m\cr\m\cr}$$
\noindent\m\par
\noindent$\m$\par
\setbox0=\vbox{\noindent\m\par}
\setbox0=\hbox{\vbox{\m}\vtop{\m}\vcenter{\m}}
$\vcenter{\m}$
\halign{#\m\cr a\cr}
\setbox0=\vbox{\halign{#\m\cr a\cr}}
\insert100{\m}\par
\noindent\vadjust{\m}\par
""" + CONDITION_TAIL)
P('ifvoid ifhbox ifvbox', r"""
\T{box conditionals}
\def\m#1{\ifvoid#1 \V{void}\else\V{not void}\fi\ifhbox#1 \V{hbox}\else\V{not hbox}\fi\ifvbox#1 \V{vbox}\else\V{not vbox}\fi}
\setbox1=\box2 \m1
\setbox1=\hbox{}\m1 \setbox1=\vbox{}\m1 \setbox1=\vtop{}\m1
\setbox1=\hbox{}\setbox2=\box1 \m1 \m2
\m{32767}
\ifvoid32768 \V{big}\fi
\ifhbox-1 \V{neg}\fi
\ifvbox x \V{no number}\fi
\ifvoid\fi
""" + CONDITION_TAIL)
P('ifx', r"""
\T{ifx}
\def\a{x}\def\b{x}\def\c{y}\def\d#1{x}\long\def\e{x}\outer\def\o{x}\protected\def\p{x}
\ifx\a\b\V{1t}\else\V{1f}\fi\ifx\a\c\V{2t}\else\V{2f}\fi\ifx\a\d\V{3t}\else\V{3f}\fi
\ifx\a\e\V{4t}\else\V{4f}\fi\ifx\a\p\V{5t}\else\V{5f}\fi
\ifx aa\V{6t}\else\V{6f}\fi\ifx ab\V{7t}\else\V{7f}\fi\ifx a\a\V{8t}\else\V{8f}\fi
\ifx\relax\relax\V{9t}\else\V{9f}\fi\let\r=\relax\ifx\r\relax\V{10t}\else\V{10f}\fi
\ifx\undefinedA\undefinedB\V{11t}\else\V{11f}\fi
\ifx{{\V{12t}\else\V{12f}\fi}}
\ifx$$\V{13t}\else\V{13f}\fi\ifx##\V{14t}\else\V{14f}\fi
\ifx~~\V{15t}\else\V{15f}\fi
\ifx\par\par\V{16t}\else\V{16f}\fi
\chardef\f=5 \chardef\g=5 \chardef\h=6 \ifx\f\g\V{17t}\else\V{17f}\fi\ifx\f\h\V{18t}\else\V{18f}\fi
\mathchardef\g=5 \ifx\f\g\V{19t}\else\V{19f}\fi
\countdef\f=5 \countdef\g=5 \ifx\f\g\V{20t}\else\V{20f}\fi
\font\f=cmr10 \font\g=cmr10 \ifx\f\g\V{21t}\else\V{21f}\fi\ifx\f\tenrm\V{22t}\else\V{22f}\fi
\ifx\f\nullfont\V{23t}\else\V{23f}\fi
\ifx\fi
\ifx a\fi
\expandafter\ifx\csname noname\endcsname\relax\V{24t}\else\V{24f}\fi
\ifx\o\o\V{outer ok}\fi
\ifx\iftrue\iffalse\V{25t}\else\V{25f}\fi
""" + CONDITION_TAIL)
P('ifeof', r"""
\T{ifeof}
\ifeof0 \V{1t}\else\V{1f}\fi\ifeof15 \V{2t}\else\V{2f}\fi
\ifeof16 \V{3t}\else\V{3f}\fi
\ifeof-1 \V{4t}\else\V{4f}\fi
\openin3=plain.tex \ifeof3 \V{5t}\else\V{5f}\fi\closein3 \ifeof3 \V{6t}\else\V{6f}\fi
\openin3=nosuchfile.xyz \ifeof3 \V{7t}\else\V{7f}\fi
\ifeof x \V{8t}\else\V{8f}\fi
\ifeof\fi
""" + CONDITION_TAIL)
P('iftrue iffalse', r"""
\T{iftrue iffalse}
\iftrue\V{1t}\else\V{1f}\fi\iffalse\V{2t}\else\V{2f}\fi
\iffalse\undefinedthing\fi
\iffalse \if\ifnum\ifx\fi\fi\fi\V{balanced}
\iffalse \else\iftrue\V{3}\fi\fi
\iftrue\iftrue\iffalse\else\V{deep}\fi\fi\fi
\let\f=\fi \iffalse\f\fi
\def\z{\iffalse}\iffalse\z\fi\fi
\iffalse\csname fi\endcsname\fi
\iffalse\par\fi
\unless\iftrue\V{4t}\else\V{4f}\fi\unless\iffalse\V{5t}\else\V{5f}\fi
\iffalse{\fi\V{brace}\iffalse}\fi
""" + CONDITION_TAIL)
P('ifcase', r"""
\T{ifcase}
\ifcase0 \V{0}\or\V{1}\or\V{2}\else\V{e}\fi
\ifcase1 \V{0}\or\V{1}\or\V{2}\else\V{e}\fi
\ifcase2 \V{0}\or\V{1}\or\V{2}\else\V{e}\fi
\ifcase3 \V{0}\or\V{1}\or\V{2}\else\V{e}\fi
\ifcase-1 \V{0}\or\V{1}\or\V{2}\else\V{e}\fi
\ifcase2 \V{0}\or\V{1}\fi
\ifcase2147483647 \V{0}\fi
\ifcase1 \ifcase1 a\or\V{in}\fi\or\V{out}\fi
\ifcase0 \iftrue\or\fi\or\V{x}\fi
\ifcase1 \V{0}\or\showifs\or\V{2}\fi
\ifcase x\V{0}\fi
\ifcase0 \else\or\fi
\ifcase\fi
\unless\ifcase0 \fi
""" + CONDITION_TAIL)
P('fi or else', r"""
\T{fi or else}
\fi
\else
\or
\iftrue\fi\fi
\iftrue\or\fi
\iftrue\else\else\fi
\iffalse\else\or\fi
\iffalse\else\else\fi
\ifcase0 \or\or\else\fi
\ifcase1 \or\else\or\fi
\let\f=\fi \let\e=\else \let\o=\or
\iftrue\e\f
\iffalse\e\V{let else}\f
\ifcase1 \o\V{let or}\f
\setbox0=\hbox{\fi}\setbox0=\hbox{\else}\setbox0=\hbox{\or}
\unless\fi
\edef\a{\fi}\show\a
\def\b{\fi}\iftrue\b
\csname fi\endcsname
\iftrue\csname fi\endcsname
\expandafter\show\csname fi\endcsname
\iftrue\expandafter\fi\V{after fi}
""" + CONDITION_TAIL)
# ---------------------------------------------------------------------------------------------- registers
REG = r"""
\T{register %(n)s}
\%(n)s0=%(a)s \showthe\%(n)s0
\%(n)s255=%(a)s \showthe\%(n)s255
\%(n)s256=%(a)s \showthe\%(n)s256
\%(n)s32767=%(a)s \showthe\%(n)s32767
\%(n)s32768=%(a)s
\%(n)s-1=%(a)s
\%(n)sx=%(a)s
\begingroup\%(n)s1000=%(a)s \showthe\%(n)s1000\endgroup\showthe\%(n)s1000
\begingroup\global\%(n)s1001=%(a)s \endgroup\showthe\%(n)s1001
\V{\the\%(n)s0}\V{\the\%(n)s999}
\advance\%(n)s0 by %(a)s \showthe\%(n)s0
\multiply\%(n)s0 by 3 \showthe\%(n)s0
\divide\%(n)s0 by 2 \showthe\%(n)s0
\divide\%(n)s0 by 0
\%(n)s1=\%(n)s0 \showthe\%(n)s1
\show\%(n)s
\%(n)s
"""
P('count', REG % {'n': 'count', 'a': '5'} + r"""
\count1=2147483647 \advance\count1 by 1
\count1=2147483647 \multiply\count1 by 2
\count1=-2147483647 \advance\count1 by-1 \showthe\count1
\count1=\dimen0 \count1=1\relax \count1=\baselineskip\showthe\count1
\count1=7 \divide\count1 by 2 \showthe\count1 \count1=-7 \divide\count1 by 2 \showthe\count1
""")
P('dimen', REG % {'n': 'dimen', 'a': '5pt'} + r"""
\dimen1=16383.99999pt \advance\dimen1 by 1sp \showthe\dimen1
\dimen1=16383.99999pt \advance\dimen1 by 1pt \showthe\dimen1
\dimen1=16383pt \multiply\dimen1 by 2 \showthe\dimen1
\dimen1=16384pt\showthe\dimen1
\dimen1=-16384pt\showthe\dimen1
\dimen1=0.1pt \showthe\dimen1 \dimen1=0.00001pt \showthe\dimen1 \dimen1=0.000001pt \showthe\dimen1 \dimen1=.3333333pt \showthe\dimen1
\dimen1=1.5\dimen1 \showthe\dimen1 \dimen1=2\baselineskip \showthe\dimen1
\dimen1=1em \showthe\dimen1 \dimen1=1ex \showthe\dimen1
\dimen1=1in \showthe\dimen1 \dimen1=1cm \showthe\dimen1 \dimen1=1mm \showthe\dimen1 \dimen1=1bp \showthe\dimen1
\dimen1=1pc \showthe\dimen1 \dimen1=1dd \showthe\dimen1 \dimen1=1cc \showthe\dimen1 \dimen1=1nd \showthe\dimen1
\dimen1=1nc \showthe\dimen1 \dimen1=1sp \showthe\dimen1 \dimen1=1px \showthe\dimen1
\dimen1=1 PT \showthe\dimen1 \dimen1=1Pt \showthe\dimen1 \dimen1=1 true pt \showthe\dimen1 \dimen1=1truecm \showthe\dimen1
\dimen1=1xx \par
\dimen1=1\relax \par
\dimen1=x \par
\dimen1=1.\showthe\dimen1 \dimen1=,5pt \showthe\dimen1 \dimen1=1,5pt \showthe\dimen1
\dimen1="10pt \showthe\dimen1 \dimen1='10pt \showthe\dimen1 \dimen1=`apt \showthe\dimen1
\dimen1=1mu \par
\dimen1=\thinmuskip \par
\dimen1=--1pt \showthe\dimen1 \dimen1=+-+1pt \showthe\dimen1
""")
P('skip', REG % {'n': 'skip', 'a': '5pt plus 1pt minus 2pt'} + r"""
\skip1=1pt plus 1fil \showthe\skip1 \skip1=1pt plus 1fill \showthe\skip1 \skip1=1pt plus 1filll \showthe\skip1
\skip1=1pt plus 1fillll \par
\skip1=1pt minus 1fil plus 1fil \par
\skip1=1pt plus 16384pt \showthe\skip1
\skip1=1pt plus 16384fil \showthe\skip1
\skip1=1pt plus -1fil minus -1fill \showthe\skip1
\advance\skip1 by 1pt plus 1fill \showthe\skip1
\skip1=\dimen0 \showthe\skip1 \dimen1=\skip1 \showthe\dimen1
\count1=\skip1 \showthe\count1
\multiply\skip1 by 2147483647 \divide\skip1 by 0
\skip1=x \par
\skip1=1pt plus \par
\skip1=1pt plus x \par
\skip1=\thinmuskip \par
\skip1=-\skip1 \showthe\skip1
""")
P('muskip', REG % {'n': 'muskip', 'a': '5mu plus 1mu minus 2mu'} + r"""
\muskip1=1mu plus 1fil minus 1fill \showthe\muskip1
\muskip1=1pt \showthe\muskip1
\muskip1=\skip0 \showthe\muskip1
\muskip1=\thinmuskip \showthe\muskip1
\muskip1=x \par
\skip1=\muskip1 \par
\multiply\muskip1 by 2147483647
""")
P('toks', r"""
\T{register toks}
\toks0={a#b\c}\showthe\toks0
\toks255={x}\showthe\toks255 \toks256={y}\showthe\toks256 \toks32767={z}\showthe\toks32767
\toks32768={w}
\toks-1={w}
\toks1=\toks0 \showthe\toks1
\toks1={\the\toks0}\showthe\toks1
\toks1\expandafter{\the\toks0 q}\showthe\toks1
\toks1=\relax
\toks1=x
\toks1=\undefinedthing
\def\m{{m}}\toks1=\expandafter\m \showthe\toks1
\toks1=\everypar\showthe\toks1
\toks1={}\showthe\toks1
\begingroup\toks2={local}\showthe\toks2\endgroup\showthe\toks2
\begingroup\global\toks2={global}\endgroup\showthe\toks2
\edef\a{\the\toks0}\show\a
\V{\the\toks0}
\show\toks
\toks0\par
\toks1={}}
""")
P('box', r"""
\T{box}
\setbox1=\hbox{x}\box1 \showbox1
\setbox1=\hbox{x}\setbox2=\hbox{\box1}\showbox2\showbox1
\setbox1=\hbox{x}\setbox2=\vbox{\box1}\showbox2
\box2
\box32767 \box32768 \box-1
\box x\par
\setbox1=\vbox{}\setbox2=\hbox{\box1}\showbox2
\setbox1=\hbox{x}\setbox2=\hbox{$\box1$}\showbox2
\setbox1=\hbox{x}\setbox2=\hbox{$\vcenter{\box1}$}\showbox2
\setbox1=\hbox{x}\setbox2=\hbox{\raise1pt\box1}\showbox2
\show\box
""")
P('copy', r"""
\T{copy}
\setbox1=\hbox{x}\copy1 \showbox1
\setbox2=\hbox{\copy1 \copy1}\showbox2
\setbox2=\vbox{\copy1 \copy1}\showbox2
\setbox2=\hbox{$\copy1$}\showbox2
\setbox1=\box3 \copy1
\copy32768 \copy-1 \copy x\par
""")
P('setbox', r"""
\T{setbox}
\setbox1=\hbox{x}\showbox1 \setbox1\hbox{y}\showbox1
\setbox32767=\vbox{z}\showbox32767
\setbox32768=\hbox{}
\setbox-1=\hbox{}
\setbox1=\relax
\setbox1=x\par
\setbox1=\hrule
\setbox1=\par
\global\setbox1=\hbox{g}
\begingroup\setbox1=\hbox{l}\endgroup\showbox1
\begingroup\global\setbox1=\hbox{m}\endgroup\showbox1
\setbox1=\box2 \showbox1
\setbox1=\copy2 \showbox1
\setbox1=\lastbox\showbox1
\setbox1=\hbox to 10pt{\hfil}\showbox1 \setbox1=\hbox spread 10pt{\hfil}\showbox1
\setbox1=\vbox to 10pt{\vfil}\showbox1 \setbox1=\vtop spread 10pt{\vfil}\showbox1
\setbox1=\hbox foo{}
\setbox1=\hbox to x{}
\setbox1=\hbox{}}
\setbox1=\hbox{\setbox2=\hbox{}}\showbox1
\setbox1=\hbox{\global\setbox2=\hbox{g}}\showbox2
\setbox1=\vsplit2 to 0pt
""")
P('hbox', r"""
\T{hbox}
\setbox1=\hbox{}\showbox1 \setbox1=\hbox to 100pt{a\hfil b}\showbox1 \setbox1=\hbox spread 10pt{a\hfil b}\showbox1
\setbox1=\hbox to 1pt{aaa}\showbox1
\setbox1=\hbox to 100pt{a}\showbox1
\setbox1=\hbox to 100pt{a\hskip0pt plus 1pt b}\showbox1
\setbox1=\hbox to 10pt{a\hskip0pt minus 1pt b}\showbox1
\setbox1=\hbox to 16383.99999pt{}\showbox1
\setbox1=\hbox spread-1pt{}\showbox1
\setbox1=\hbox{\vrule\hrule\par}\showbox1
\setbox1=\hbox{\vskip1pt}\showbox1
\setbox1=\hbox{\unvbox2}\showbox1
\setbox1=\hbox{\vfil}\showbox1
\setbox1=\hbox{\moveleft 1pt\hbox{}}\showbox1
\setbox1=\hbox{\raise 1pt\vbox{}\lower 1pt\hbox{}}\showbox1
\hbox to 10pt{}
\hbox\relax\par
\hbox{}}
\hbox spread 1pt plus 1pt{}\par
\everyhbox={\V{everyhbox ran}}\hbox{}\everyhbox={}
""")
P('vbox', r"""
\T{vbox}
\setbox1=\vbox{}\showbox1 \setbox1=\vbox to 100pt{a\vfil b}\showbox1 \setbox1=\vbox spread 10pt{a\vfil b}\showbox1
\setbox1=\vbox to 1pt{\hrule height 5pt}\showbox1
\setbox1=\vbox to 100pt{\hrule}\showbox1
\setbox1=\vbox{\hbox{a}\hbox{b}}\showbox1
\setbox1=\vbox{\hrule depth 5pt}\showbox1
\begingroup\boxmaxdepth=1pt \setbox1=\vbox{\hrule depth 5pt}\showbox1 \endgroup
\setbox1=\vbox{\moveleft 1pt\hbox{}\moveright 1pt\vbox{}}\showbox1
\setbox1=\vbox{\raise 1pt\hbox{}}\showbox1
\setbox1=\vbox{\hfil}\showbox1
\setbox1=\vbox{a\par}\showbox1
\setbox1=\vbox{\unhbox2}\showbox1
\vbox{}
\vbox\relax\par
\everyvbox={\V{everyvbox ran}}\setbox1=\vbox{}\everyvbox={}
\vbox to 10pt{}
""")
P('vtop', r"""
\T{vtop}
\setbox1=\vtop{}\showbox1 \setbox1=\vtop to 100pt{\hbox{a}\vfil\hbox{b}}\showbox1 \setbox1=\vtop spread 10pt{a\vfil b}\showbox1
\setbox1=\vtop{\hrule height 1pt depth 2pt\hbox{b}}\showbox1
\setbox1=\vtop{\kern3pt\hbox{a}}\showbox1
\setbox1=\vtop{\hbox{a}\kern3pt\hbox{b}}\showbox1
\setbox1=\vtop{\vbox{\hbox{a}}}\showbox1
\vtop{}
\vtop\relax\par
\everyvbox={\V{everyvbox ran}}\setbox1=\vtop{}\everyvbox={}
""")
P('lastbox', r"""
\T{lastbox}
\setbox1=\hbox{a\hbox{b}\setbox2=\lastbox\showbox2}\showbox1
\setbox1=\vbox{\hbox{a}\hbox{b}\setbox2=\lastbox\showbox2 \setbox2=\lastbox\showbox2 \setbox2=\lastbox\showbox2}\showbox1
\setbox1=\hbox{\kern1pt\setbox2=\lastbox\showbox2}
\setbox2=\lastbox
\lastbox
\setbox1=\hbox{$\setbox2=\lastbox$}
\setbox1=\hbox{\moveleft 3pt\hbox{m}\setbox2=\lastbox\showbox2}
\setbox1=\vbox{\moveleft 3pt\hbox{m}\setbox2=\lastbox\showbox2}
\setbox1=\hbox{\raise 3pt\hbox{m}\setbox2=\lastbox\showbox2}
\setbox1=\hbox{\hbox{a}\lastbox\lastbox}\showbox1
\noindent\hbox{x}\setbox2=\lastbox\showbox2\par
\setbox1=\vbox{\noindent\setbox2=\lastbox\showbox2\par}
""")
P('vsplit', r"""
\T{vsplit}
\def\f{\hbox{a}\penalty0 \hbox{b}\penalty0 \hbox{c}\penalty0 \hbox{d}}
\setbox1=\vbox{\f}\setbox2=\vsplit1 to 20pt \showbox2 \showbox1
\setbox1=\vbox{\f}\setbox2=\vsplit1 to 0pt \showbox2 \showbox1
\setbox1=\vbox{\f}\setbox2=\vsplit1 to 1000pt \showbox2 \showbox1
\setbox1=\hbox{a}\setbox2=\vsplit1 to 1pt
\setbox1=\box3 \setbox2=\vsplit1 to 1pt \showbox2
\setbox1=\vbox{\mark{m1}\hbox{a}\penalty0 \mark{m2}\hbox{b}\mark{m3}\penalty0 \hbox{c}}
\setbox2=\vsplit1 to 10pt \V{[\splitfirstmark][\splitbotmark]}
\setbox2=\vsplit1 to 10pt \V{[\splitfirstmark][\splitbotmark]}
\setbox1=\vbox{\insert100{x}\hbox{a}}\setbox2=\vsplit1 to 0pt
\begingroup\splittopskip=5pt \splitmaxdepth=1pt \setbox1=\vbox{\f}\setbox2=\vsplit1 to 12pt \showbox2\showbox1\endgroup
\vsplit1 to 1pt
\setbox2=\vsplit1 to
\setbox2=\vsplit x to 1pt
\setbox2=\vsplit32768 to 1pt
\setbox2=\hbox{\vsplit3 to 1pt}\showbox2
""")
P('unhbox unhcopy', r"""
\T{unhbox unhcopy}
\setbox1=\hbox{a\kern1pt b}\setbox2=\hbox{\unhcopy1 \unhcopy1 \unhbox1 \unhbox1}\showbox2\showbox1
\setbox1=\hbox{a}\noindent\unhbox1\par
\setbox1=\hbox{a}\unhbox1\par
\setbox1=\hbox{a}\unhcopy1\par
\setbox1=\vbox{a}\setbox2=\hbox{\unhbox1}\showbox2
\setbox1=\vbox{a}\setbox2=\hbox{\unhcopy1}\showbox2
\setbox1=\hbox{a}\setbox2=\vbox{\unhbox1}\showbox2
\setbox1=\hbox{a}\setbox2=\hbox{$\unhbox1$}\showbox2
\setbox1=\box3 \setbox2=\hbox{$\unhbox1$}\showbox2
\setbox1=\hbox{\hskip 3pt plus 1fil x\hskip 1pt}\setbox2=\hbox{\unhbox1 \unskip\setbox3=\lastbox}\showbox2
\unhbox32768 \unhbox-1 \unhbox x
\par
""")
P('unvbox unvcopy', r"""
\T{unvbox unvcopy}
\setbox1=\vbox{\hbox{a}\kern1pt\hbox{b}}\setbox2=\vbox{\unvcopy1 \unvcopy1 \unvbox1 \unvbox1}\showbox2\showbox1
\setbox1=\vbox{\hbox{a}}\unvbox1
\setbox1=\vbox{\hbox{a}}\unvcopy1
\setbox1=\hbox{a}\setbox2=\vbox{\unvbox1}\showbox2
\setbox1=\hbox{a}\setbox2=\vbox{\unvcopy1}\showbox2
\setbox1=\vbox{a}\setbox2=\hbox{\unvbox1}\showbox2
\setbox1=\vbox{a}\setbox2=\hbox{$\unvbox1$}\showbox2
\setbox1=\vbox{\hbox{a}\penalty5 \kern1pt\hskip1pt}\setbox2=\vbox{\unvbox1 \unskip\unkern\unpenalty}\showbox2
\unvbox32768 \unvbox-1 \unvbox x
\par
""")
P('wd ht dp', r"""
\T{box dimensions}
\setbox1=\hbox{xyg}\V{\the\wd1 \the\ht1 \the\dp1}
\wd1=100pt \ht1=20pt \dp1=5pt \showbox1 \V{\the\wd1 \the\ht1 \the\dp1}
\wd1=-1pt \ht1=-1pt \dp1=-1pt \showbox1
\wd3=1pt \ht3=1pt \dp3=1pt \showthe\wd3 \showthe\ht3 \showthe\dp3
\wd32767=1pt \wd32768=1pt \wd-1=1pt \ht32768=1pt \dp32768=1pt
\wd1=16384pt
\advance\wd1 by 1pt \showthe\wd1 \multiply\ht1 by 2 \showthe\ht1 \divide\dp1 by 2 \showthe\dp1
\begingroup\wd1=7pt \endgroup\showthe\wd1
\begingroup\global\wd1=8pt \endgroup\showthe\wd1
\wd x
\wd1 \par
\dimen1=\wd1 \showthe\dimen1 \count1=\wd1 \showthe\count1
\ifdim\wd1>0pt \V{positive}\fi
""")
P('raise lower moveleft moveright', r"""
\T{box moves}
\setbox1=\hbox{\raise 2pt\hbox{a}\lower 2pt\hbox{b}\raise-1pt\vbox{}\lower-1pt\vtop{}}\showbox1
\setbox1=\vbox{\moveleft 2pt\hbox{a}\moveright 2pt\hbox{b}\moveleft-1pt\vbox{}\moveright-1pt\vtop{}}\showbox1
\setbox1=\vbox{\raise 2pt\hbox{a}}\showbox1
\setbox1=\hbox{\moveleft 2pt\hbox{a}}\showbox1
\setbox1=\hbox{$\raise 2pt\hbox{a}\lower 1pt\vbox{}$}\showbox1
\raise 2pt\hbox{a}\par
\lower 2pt\hbox{a}\par
\moveleft 2pt\hbox{a}\moveright 2pt\vbox{}
\setbox1=\hbox{\raise 16384pt\hbox{}}
\setbox1=\hbox{\raise x\hbox{}}
\setbox1=\hbox{\raise 1pt x}
\setbox1=\hbox{\raise 1pt\relax}
\setbox2=\hbox{y}\setbox1=\hbox{\raise 1pt\box2 \lower 1pt\copy3}\showbox1
\setbox1=\hbox{\raise 1pt\lastbox}\showbox1
\setbox1=\hbox{\raise 1pt\vsplit3 to 0pt}\showbox1
\moveleft 1pt\relax
""")
# ---------------------------------------------------------------------------------------------- spacing, kerning, rules
P('hskip hfil hfill hss hfilneg', r"""
\T{horizontal glue}
\setbox1=\hbox to 100pt{a\hfil b\hfill c\hss d\hfilneg e\hskip 1pt plus 1pt minus 1pt f}\showbox1
\setbox1=\hbox to 100pt{\hfil\hfil}\showbox1
\setbox1=\hbox to 1pt{\hss xxxx\hss}\showbox1
\setbox1=\hbox to 100pt{\hfil\hfilneg\hfil}\showbox1
\setbox1=\hbox to 100pt{a\hskip 0pt plus 1filll b\hskip 0pt plus 1fill c}\showbox1
\setbox1=\hbox{$a\hfil b\hskip1pt c$}\showbox1
\hskip 1pt\par
\hfil\par
\hfill\par
\hss\par
\hfilneg\par
\setbox1=\vbox{\hskip 1pt\par}\showbox1
\setbox1=\vbox{\hfil\par}\showbox1
\setbox1=\hbox{\hskip x}
\setbox1=\hbox{\hskip 1pt plus}
\setbox1=\hbox{\hskip 16384pt}
\setbox1=\hbox{\hskip\thinmuskip}
""")
P('vskip vfil vfill vss vfilneg', r"""
\T{vertical glue}
\setbox1=\vbox to 100pt{a\vfil b\vfill c\vss d\vfilneg e\vskip 1pt plus 1pt minus 1pt f}\showbox1
\setbox1=\vbox to 100pt{\vfil\vfil}\showbox1
\setbox1=\vbox to 1pt{\vss\hbox{x}\vss}\showbox1
\vskip 1pt\vfil\vfill\vss\vfilneg
\setbox1=\hbox{\vskip 1pt\vfil\vfill\vss\vfilneg}\showbox1
\setbox1=\hbox{$\vskip1pt$}
\setbox1=\vbox{\vskip x}
\setbox1=\vbox{\vskip 16384pt}
\noindent a\vskip 1pt b\par
""")
P('kern', r"""
\T{kern}
\setbox1=\hbox{a\kern 1pt b\kern-1pt c\kern0pt}\showbox1
\setbox1=\vbox{\kern 1pt\hbox{}\kern-1pt}\showbox1
\setbox1=\hbox{$\kern 1pt a$}\showbox1
\kern1pt
\setbox1=\hbox{\kern x}
\setbox1=\hbox{\kern 16384pt}
\setbox1=\hbox{\kern\thinmuskip}
\setbox1=\hbox{\kern 1mu}
\setbox1=\hbox{a\kern 1pt\kern 2pt\unkern\unkern\unkern}\showbox1
""")
P('penalty', r"""
\T{penalty}
\setbox1=\hbox{a\penalty 5 b\penalty-10000 c\penalty 10000}\showbox1
\setbox1=\vbox{\penalty 5 \hbox{}\penalty-20000}\showbox1
\setbox1=\hbox{$a\penalty 5 b$}\showbox1
\penalty 5
\penalty 2147483647 \penalty -2147483647
\penalty 2147483648
\penalty x\par
\setbox1=\vbox{\penalty 3 \showthe\lastpenalty\unpenalty\showthe\lastpenalty}
""")
P('unskip unkern unpenalty', r"""
\T{remove last item}
\setbox1=\hbox{a\hskip1pt\unskip\kern1pt\unkern\penalty1 \unpenalty}\showbox1
\setbox1=\vbox{\hbox{}\vskip1pt\unskip\kern1pt\unkern\penalty1 \unpenalty}\showbox1
\setbox1=\hbox{\kern1pt\unskip\unpenalty}\showbox1
\setbox1=\hbox{\hskip1pt\unkern\unpenalty}\showbox1
\setbox1=\hbox{\penalty1 \unskip\unkern}\showbox1
\setbox1=\hbox{\unskip\unkern\unpenalty}\showbox1
\setbox1=\hbox{$a\hskip1pt\unskip\kern1pt\unkern\penalty1 \unpenalty$}\showbox1
\unskip
\unkern
\unpenalty
\noindent\unskip\unkern\unpenalty\par
\hbox{}\vskip1pt\unskip\unskip\kern1pt\unkern\penalty1 \unpenalty
""")
P('hrule', r"""
\T{hrule}
\setbox1=\vbox{\hrule\hrule width 1pt\hrule height 1pt\hrule depth 1pt\hrule width 1pt height 2pt depth 3pt}\showbox1
\setbox1=\vbox{\hrule height 1pt height 2pt}\showbox1
\setbox1=\vbox{\hrule width-1pt height-1pt depth-1pt}\showbox1
\setbox1=\hbox{\hrule}\showbox1
\setbox1=\hbox{$\hrule$}\showbox1
\hrule\par
\noindent\hrule\par
\setbox1=\vbox{\hrule width x}
\setbox1=\vbox{\hrule foo}
\setbox1=\vbox{\hrule width 16384pt}
""")
P('vrule', r"""
\T{vrule}
\setbox1=\hbox{\vrule\vrule width 1pt\vrule height 1pt\vrule depth 1pt\vrule width 1pt height 2pt depth 3pt}\showbox1
\setbox1=\hbox{\vrule width 1pt width 2pt}\showbox1
\setbox1=\hbox{\vrule width-1pt height-1pt depth-1pt}\showbox1
\setbox1=\hbox{$\vrule$}\showbox1
\setbox1=\vbox{\vrule\par}\showbox1
\vrule\par
\setbox1=\hbox{\vrule width x}
\setbox1=\hbox{\vrule foo}
""")
P('leaders cleaders xleaders', r"""
\T{leaders}
\def\l#1{\setbox1=\hbox to 100pt{a#1\hrule\hfil b#1\vrule width 1pt\hskip 20pt plus 1fil c#1\hbox to 7pt{.\hfil}\hfil d#1\vbox{\hbox{x}}\hskip 30pt}\showbox1
  \shipout\copy1 }
\l\leaders\l\cleaders\l\xleaders
\setbox1=\vbox to 100pt{\hbox{a}\leaders\hrule\vfil\hbox{b}\cleaders\vbox to 7pt{\hbox{.}\vfil}\vfil\xleaders\hbox{.}\vfil}\showbox1\shipout\copy1
\setbox1=\hbox{$a\leaders\hrule\hskip1pt b\leaders\hrule\mskip 1mu c$}\showbox1
\setbox1=\hbox{\leaders x}
\setbox1=\hbox{\leaders\hrule\kern1pt}
\setbox1=\hbox{\leaders\hbox{}}
\setbox1=\vbox{\leaders\hrule\hfil}
\leaders\hrule\vskip 1pt
\leaders\hbox{}\hskip1pt\par
""")
P('discretionary -', r"""
\T{discretionary}
\setbox1=\hbox{a\discretionary{b}{c}{d}e\-f\discretionary{}{}{}}\showbox1
\setbox1=\hbox{\discretionary{\kern1pt\hbox{}\vrule}{\penalty0}{\accent19 e\char5}}\showbox1
\setbox1=\hbox{\discretionary{\hskip1pt}{}{}}\showbox1
\setbox1=\hbox{\discretionary{}{\vskip1pt}{}}\showbox1
\setbox1=\hbox{\discretionary{}{}{\mark{m}\insert100{}}}\showbox1
\setbox1=\hbox{$a\discretionary{b}{c}{}d\-$}\showbox1
\setbox1=\hbox{$a\discretionary{b}{c}{d}$}\showbox1
\hyphenchar\tenrm=-1 \setbox1=\hbox{a\-b}\showbox1 \hyphenchar\tenrm=45
\hyphenchar\tenrm=256 \setbox1=\hbox{a\-b}\showbox1 \hyphenchar\tenrm=45
\discretionary{}{}{}\par
\-\par
\setbox1=\vbox{\-\discretionary{}{}{}}\showbox1
\discretionary x\par
\setbox1=\hbox{\discretionary{}x}\par
\setbox1=\hbox{\discretionary{}{}x}\par
\setbox1=\hbox{\discretionary{a}{b}{c}\unskip\unpenalty\unkern}\showbox1
\setbox1=\hbox{\discretionary{\aaaaa}{}{}}
""")
P('/', r"""
\T{italic correction}
\setbox1=\hbox{\it f\/}\showbox1
\setbox1=\hbox{\it f\/\/}\showbox1
\setbox1=\hbox{\/}\showbox1
\setbox1=\hbox{\kern1pt\/}\showbox1
\setbox1=\hbox{\it fi\/}\showbox1
\setbox1=\hbox{\it f\hbox{}\/}\showbox1
\setbox1=\hbox{$\it f\/$}\showbox1
\/\par
\noindent\/\par
\setbox1=\vbox{\/}\showbox1
\setbox1=\hbox{\nullfont x\/}\showbox1
""")
P(' ', r"""
\T{control space}
\setbox1=\hbox{a\ b\  c}\showbox1
\setbox1=\hbox{\spaceskip=5pt a\ b}\showbox1
\setbox1=\hbox{\xspaceskip=5pt \spacefactor=3000 \ b}\showbox1
\setbox1=\hbox{$a\ b$}\showbox1
\ \par
\setbox1=\vbox{\ \par}\showbox1
\setbox1=\hbox{\nullfont\ x}\showbox1
\def\ {x}\show\ \V{\ }
""")
P('noboundary', r"""
\T{noboundary}
\setbox1=\hbox{f\noboundary i}\showbox1
\setbox1=\hbox{\noboundary fi}\showbox1
\setbox1=\hbox{fi\noboundary}\showbox1
\setbox1=\hbox{$\noboundary a$}\showbox1
\noboundary\par
\noindent\noboundary\par
\setbox1=\hbox{\noboundary\noboundary}\showbox1
""")
P('char', r"""
\T{char}
\setbox1=\hbox{\char65 \char"42 \char`C\char255}\showbox1
\setbox1=\hbox{\char256}
\setbox1=\hbox{\char-1}
\setbox1=\hbox{\char x}
\setbox1=\hbox{$\char65$}\showbox1
\setbox1=\hbox{\nullfont\char65}\showbox1
\setbox1=\hbox{\tracinglostchars=1 \char200 \tracinglostchars=2 \char201 \tracinglostchars=3 \char202 }\showbox1
\setbox1=\hbox{\char`f\char`i\char`f\char`l}\showbox1
\char65\par
\setbox1=\hbox{f\char`i}\showbox1
""")
P('accent', r"""
\T{accent}
\setbox1=\hbox{\accent127 e\accent19 a\accent18 o}\showbox1
\setbox1=\hbox{\accent127 \relax e}\showbox1
\setbox1=\hbox{\accent127 \hbox{}}\showbox1
\setbox1=\hbox{\accent127\par}\showbox1
\setbox1=\hbox{\accent127\char65}\showbox1
\setbox1=\hbox{\accent127\font\x=cmti10 \x e}\showbox1
\setbox1=\hbox{\tenit\accent19 \tenrm e}\showbox1
\setbox1=\hbox{\accent256 e}
\setbox1=\hbox{\accent-1 e}
\setbox1=\hbox{\accent200 e}\showbox1
\setbox1=\hbox{\accent127 \kern1pt e}\showbox1
\setbox1=\hbox{$\accent127 e$}\showbox1
\accent127 e\par
\accent127
\relax\par
""")
P('par', r"""
\T{par}
\par\par
\setbox1=\hbox{\par}\showbox1
\setbox1=\vbox{\par}\showbox1
\setbox1=\vbox{a\par\par b\par}\showbox1
\setbox1=\hbox{$\par$}\showbox1
\everypar={\V{everypar}}\noindent a\par\everypar={}
\def\par{\V{redefined par}\endgraf}\noindent b\par\let\par=\endgraf
\catcode`\^^M=5
\noindent c

\noindent d
\def\a#1{#1}\a{\par}
\a\par
\let\p=\par\noindent e\p
\setbox1=\vbox{\parfillskip=0pt \hsize=5pt \noindent x\par}\showbox1
\setbox1=\vbox{\hbox{a}\par}\showbox1
\showlists
""")
P('indent noindent quitvmode', r"""
\T{indent noindent quitvmode}
\indent a\par
\noindent b\par
\quitvmode c\par
\indent\indent d\par
\noindent\noindent e\par
\quitvmode\quitvmode f\par
\noindent\indent g\par
\setbox1=\hbox{\indent x\noindent y\quitvmode z}\showbox1
\setbox1=\hbox{$\indent x\noindent y\quitvmode z$}\showbox1
\setbox1=\vbox{\indent a\par\noindent b\par\quitvmode c\par}\showbox1
\parindent=5pt \indent h\par \parindent=0pt \indent i\par \parindent=10pt
\everypar={\V{ep}}\indent\par\noindent\par\quitvmode\par\everypar={}
\showlists
""")
P('vadjust', r"""
\T{vadjust}
\noindent a\vadjust{\kern1pt}b\par
\noindent a\vadjust pre{\kern1pt}b\par
\vadjust{}
\setbox1=\hbox{\vadjust{x}}\showbox1
\setbox1=\vbox{\noindent a\vadjust{\hrule}\par}\showbox1
\setbox1=\vbox{\noindent a\vadjust pre{\hrule}\vadjust{\hbox{x}}\par}\showbox1
\setbox1=\hbox{$\vadjust{x}$}\showbox1
\noindent$\vadjust{\kern2pt}a$\par
\noindent\vadjust x\par
\noindent\vadjust{\hbox{}\par}\par
\noindent\vadjust{}}\par
\noindent\vadjust{\showlists}\par
""")
P('insert', r"""
\T{insert}
\insert100{\hbox{a}}\insert100{}\insert254{\hbox{b}}
\insert255{}
\insert256{}
\insert-1{}
\insert x{}
\insert100 x\par
\setbox1=\vbox{\insert100{}\hbox{}}\showbox1
\setbox1=\hbox{\insert100{}}\showbox1
\setbox1=\hbox{$\insert100{}$}\showbox1
\noindent a\insert100{\hbox{in par}}b\par
\insert100{\vskip 1pt plus 1fil\penalty5}
\insert100{\floatingpenalty=5 \splittopskip=1pt \splitmaxdepth=1pt \showlists}
\vfill\penalty-10000
\T{after insert page}
""")
P('mark', r"""
\T{mark}
\mark{a}\mark{b#}\mark{\relax}
\def\m{M}\mark{\m}\mark{\noexpand\m}
\setbox1=\vbox{\mark{x}}\showbox1
\setbox1=\hbox{\mark{x}}\showbox1
\setbox1=\hbox{$\mark{x}$}\showbox1
\noindent a\mark{in par}b\par
\mark x\par
\mark{}}
\vfill\penalty-10000
\V{[\topmark][\firstmark][\botmark]}
""")
P('topmark firstmark botmark splitfirstmark splitbotmark', r"""
\T{mark classes}
\V{[\topmark][\firstmark][\botmark][\splitfirstmark][\splitbotmark]}
\hbox{a}\mark{one}\hbox{b}\mark{two}\vfill\penalty-10000
\V{[\topmark][\firstmark][\botmark]}
\hbox{c}\mark{three}\vfill\penalty-10000
\V{[\topmark][\firstmark][\botmark]}
\hbox{d}\vfill\penalty-10000
\V{[\topmark][\firstmark][\botmark]}
\setbox1=\vbox{\hbox{a}\mark{s1}\penalty0 \hbox{b}\mark{s2}\penalty0 \hbox{c}\mark{s3}}
\setbox2=\vsplit1 to 12pt \V{[\splitfirstmark][\splitbotmark]}
\setbox2=\vsplit1 to 12pt \V{[\splitfirstmark][\splitbotmark]}
\setbox2=\vsplit1 to 1000pt \V{[\splitfirstmark][\splitbotmark]}
\setbox1=\box3 \setbox2=\vsplit1 to 1pt \V{[\splitfirstmark][\splitbotmark]}
\edef\a{\topmark\firstmark\botmark\splitfirstmark\splitbotmark}\show\a
\topmark\par\firstmark\par\botmark\par\splitfirstmark\par\splitbotmark\par
\show\topmark \show\firstmark \show\botmark \show\splitfirstmark \show\splitbotmark
""")
# ---------------------------------------------------------------------------------------------- catcodes and codes
P('catcode', r"""
\T{catcode}
\showthe\catcode`a\showthe\catcode`\\\showthe\catcode`\{\showthe\catcode`\%\showthe\catcode`\^^M\showthe\catcode0 \showthe\catcode255
\catcode`Q=12 \show Q\catcode`Q=11
\catcode`Q=13 \defQ{x}\show Q\catcode`Q=11
\catcode`Q=0 Qshow\relax \catcode`Q=11
\catcode`Q=1 \catcode`W=2 \setbox1=\hbox QaW\showbox1 \catcode`Q=11 \catcode`W=11
\catcode`Q=3 \setbox1=\hbox{QaQ}\showbox1 \catcode`Q=11
\catcode`Q=7 \catcode`W=8 \setbox1=\hbox{$xQ2W1$}\showbox1 \catcode`Q=11 \catcode`W=11
\catcode`Q=10 aQb\par \catcode`Q=11
\catcode`Q=14 a Q comment \par
\catcode`Q=11
\catcode`Q=9 \setbox1=\hbox{aQb}\showbox1 \catcode`Q=11
\catcode`Q=15 Q
\catcode`Q=11
\catcode`Q=4 \catcode`Q=6 \catcode`Q=5 \catcode`Q=11
\catcode`Q=16
\catcode`Q=-1
\catcode256=1
\catcode-1=1
\catcode x=1
\begingroup\catcode`Q=12 \endgroup\showthe\catcode`Q
\begingroup\global\catcode`Q=12 \endgroup\showthe\catcode`Q \catcode`Q=11
\advance\catcode`Q by 1
\V{^^41^^5a^^7a^^80^^ff^^?^^@}
\V{\string^^M}
""")
P('lccode uccode', r"""
\T{lccode uccode}
\showthe\lccode`A\showthe\uccode`a\showthe\lccode`a\showthe\uccode`A\showthe\lccode0 \showthe\uccode255
\lccode`A=`b \showthe\lccode`A\lowercase{\def\a{A}}\show\a\lccode`A=`a
\uccode`a=`C \showthe\uccode`a\uppercase{\def\a{a}}\show\a\uccode`a=`A
\lccode`A=256
\lccode`A=-1
\lccode256=1
\uccode`a=256
\uccode-1=1
\begingroup\lccode`A=5 \endgroup\showthe\lccode`A
\lccode`A=0 \setbox1=\vbox{\hsize=10pt\noindent Athematics\par}\showbox1 \lccode`A=`a
""")
P('sfcode', r"""
\T{sfcode}
\showthe\sfcode`a\showthe\sfcode`A\showthe\sfcode`.\showthe\sfcode`)
\sfcode`a=3000 \setbox1=\hbox{a b}\showbox1 \sfcode`a=1000
\sfcode`a=0 \setbox1=\hbox{a b}\showbox1 \sfcode`a=1000
\sfcode`a=32767 \setbox1=\hbox{a b}\showbox1 \sfcode`a=1000
\sfcode`a=32768
\sfcode`a=-1
\sfcode256=1
\setbox1=\hbox{A. b}\showbox1
\setbox1=\hbox{a.) b}\showbox1
\setbox1=\hbox{\frenchspacing a. b}\showbox1
\setbox1=\hbox{\nonfrenchspacing a. b}\showbox1
\begingroup\sfcode`a=5 \endgroup\showthe\sfcode`a
""")
P('mathcode', r"""
\T{mathcode}
\showthe\mathcode`a\showthe\mathcode`+\showthe\mathcode`1\showthe\mathcode`'
\mathcode`a="8000 \catcode`~=13 \def~{Z}\mathcode`~="8000 \setbox1=\hbox{$~$}\showbox1 \mathcode`a="7161
\mathcode`a="8001
\mathcode`a=-1
\mathcode256=1
\mathcode`a="0141 \setbox1=\hbox{$a$}\showbox1 \mathcode`a="7161
\mathcode`a="7161 \setbox1=\hbox{\fam1 $a$}\showbox1 \fam=-1
\setbox1=\hbox{$\fam=1 a\fam=0 a\fam=-1 a\fam=16 a$}\showbox1
\begingroup\mathcode`a=5 \endgroup\showthe\mathcode`a
\mathcode`A="8000 \setbox1=\hbox{$A$}\showbox1 \mathcode`A="7141
""")
P('delcode', r"""
\T{delcode}
\showthe\delcode`(\showthe\delcode`.\showthe\delcode`a\showthe\delcode0
\delcode`a="161300 \setbox1=\hbox{$\left a\right.$}\showbox1 \delcode`a=-1
\setbox1=\hbox{$\left a\right.$}\showbox1
\delcode`a="1000000
\delcode`a=-1 \showthe\delcode`a
\delcode`a=-2 \showthe\delcode`a
\delcode256=1
\delcode`.=0 \setbox1=\hbox{$\left.\right.$}\showbox1 \delcode`.=-1
\begingroup\delcode`a=5 \endgroup\showthe\delcode`a
""")
# ---------------------------------------------------------------------------------------------- math
P('delimiter', r"""
\T{delimiter}
\setbox1=\hbox{$\delimiter"4266308 a$}\showbox1
\setbox1=\hbox{$\left\delimiter"4266308 a\right\delimiter"5267309$}\showbox1
\setbox1=\hbox{$\delimiter"8000000$}
\setbox1=\hbox{$\delimiter-1$}
\setbox1=\hbox{\delimiter"4266308}\showbox1
\delimiter"4266308\par
""")
P('radical', r"""
\T{radical}
\setbox1=\hbox{$\radical"270370 a$}\showbox1
\setbox1=\hbox{$\radical"270370 {a+b\over c}$}\showbox1
\setbox1=\hbox{$\scriptstyle\radical"270370 a$}\showbox1
\setbox1=\hbox{$\displaystyle\radical"270370 a$}\showbox1
\setbox1=\hbox{$\radical"10000000 a$}
\setbox1=\hbox{$\radical-1 a$}
\setbox1=\hbox{$\radical"270370 $}
\setbox1=\hbox{\radical"270370 a}\showbox1
\radical"270370 a\par
\setbox1=\hbox{$\radical 0 a$}\showbox1
""")
P('mathaccent', r"""
\T{mathaccent}
\setbox1=\hbox{$\mathaccent"7014 a$}\showbox1
\setbox1=\hbox{$\mathaccent"7014 {a+b}$}\showbox1
\setbox1=\hbox{$\mathaccent"7014 \mathaccent"7015 a$}\showbox1
\setbox1=\hbox{$\scriptstyle\mathaccent"7014 a$}\showbox1
\setbox1=\hbox{$\mathaccent"7014 a^2_1$}\showbox1
\setbox1=\hbox{$\mathaccent"8000 a$}
\setbox1=\hbox{$\mathaccent-1 a$}
\setbox1=\hbox{\mathaccent"7014 a}\showbox1
\mathaccent"7014 a\par
\setbox1=\hbox{$\mathaccent"017F a$}\showbox1
\skewchar\teni=127 \setbox1=\hbox{$\mathaccent"7014 a$}\showbox1 \skewchar\teni=-1 \setbox1=\hbox{$\mathaccent"7014 a$}\showbox1
\skewchar\teni=127
""")
P('mathchar', r"""
\T{mathchar}
\setbox1=\hbox{$\mathchar"0141 \mathchar"7141$}\showbox1
\setbox1=\hbox{$\mathchar"8000$}
\setbox1=\hbox{$\mathchar-1$}
\setbox1=\hbox{$\mathchar"7141^{\mathchar"0142}_\mathchar"0143$}\showbox1
\setbox1=\hbox{\mathchar"0141}\showbox1
\mathchar"0141\par
\setbox1=\hbox{$\fam=1 \mathchar"7041$}\showbox1 \fam=-1
\setbox1=\hbox{$\mathchar"0F41$}\showbox1
""")
P('mathchoice', r"""
\T{mathchoice}
\def\c{\mathchoice{d}{t}{s}{u}}
\setbox1=\hbox{$\c^{\c^{\c}}$}\showbox1
\setbox1=\hbox{$\displaystyle\c\textstyle\c\scriptstyle\c\scriptscriptstyle\c$}\showbox1
\setbox1=\hbox{$\mathchoice{}{}{}{}$}\showbox1
\setbox1=\hbox{$\mathchoice{\hbox{a}}{\hbox{b}}{\vbox{}}{\kern1pt}$}\showbox1
\setbox1=\hbox{$\mathchoice x$}
\setbox1=\hbox{$\mathchoice{a}x$}
\setbox1=\hbox{\mathchoice{a}{b}{c}{d}}\showbox1
\mathchoice{a}{b}{c}{d}\par
\setbox1=\hbox{$\mathchoice{a}{b}{c}}
\setbox1=\hbox{$a\over\mathchoice{a}{b}{c}{d}$}\showbox1
""")
P('mathord mathop mathbin mathrel mathopen mathclose mathpunct mathinner underline overline', r"""
\T{math classes}
\def\c#1{\setbox1=\hbox{$#1a#1{a+b}#1\hbox{x}#1\vbox{}^2_1$}\showbox1
  \setbox1=\hbox{$\displaystyle a#1b+#1{c}$}\showbox1
  \setbox1=\hbox{$\scriptstyle a#1b$}\showbox1
  \setbox1=\hbox{$\scriptscriptstyle a#1b$}\showbox1
  \setbox1=\hbox{#1a}\showbox1
  \setbox1=\hbox{$#1$}\showbox1
  \setbox1=\hbox{$#1x$}\showbox1 \noindent\par}
\c\mathord
\c\mathop
\c\mathbin
\c\mathrel
\c\mathopen
\c\mathclose
\c\mathpunct
\c\mathinner
\c\underline
\c\overline
\mathord a\par \mathop a\par \mathbin a\par \mathrel a\par \mathopen a\par \mathclose a\par \mathpunct a\par \mathinner a\par \underline a\par \overline a\par
\setbox1=\hbox{$a+b=c,\ (d)\,\mathop{\rm lim}_x^y\limits\ \sum_1^2\mathinner{\ldotp}$}\showbox1
""")
P('displaylimits limits nolimits', r"""
\T{limits}
\def\c#1{\setbox1=\hbox{$\displaystyle\sum#1_1^2\mathop{a}#1_1^2$}\showbox1
  \setbox1=\hbox{$\textstyle\sum#1_1^2\mathop{a}#1_1^2$}\showbox1 }
\c\relax \c\limits \c\nolimits \c\displaylimits
\setbox1=\hbox{$\limits$}
\setbox1=\hbox{$a\limits$}
\setbox1=\hbox{$\nolimits$}
\setbox1=\hbox{$\displaylimits$}
\setbox1=\hbox{$\mathop{a}\limits\nolimits\displaylimits_1$}\showbox1
\setbox1=\hbox{\limits}\showbox1
\limits\par
\setbox1=\hbox{$\mathop{\hbox{x}}\limits^{\hbox{y}}_{\hbox{z}}$}\showbox1
\setbox1=\hbox{$\mathop{\mathchar"1350}\limits_1$}\showbox1
\setbox1=\hbox{$\mathop{\mathchar"1350}\nolimits_1$}\showbox1
""")
P('displaystyle textstyle scriptstyle scriptscriptstyle', r"""
\T{math styles}
\setbox1=\hbox{$\displaystyle a^2\textstyle a^2\scriptstyle a^2\scriptscriptstyle a^2$}\showbox1
\setbox1=\hbox{$\scriptstyle{\displaystyle a}\textstyle b^{\displaystyle c}$}\showbox1
\setbox1=\hbox{$a\over\displaystyle b$}\showbox1
\setbox1=\hbox{\displaystyle}\showbox1
\displaystyle\par
\textstyle\par
\scriptstyle\par
\scriptscriptstyle\par
$$\scriptstyle a\over\textstyle b$$
\setbox1=\hbox{$\left(\displaystyle a\over b\right)$}\showbox1
""")
P('nonscript', r"""
\T{nonscript}
\setbox1=\hbox{$a\nonscript\hskip1pt b^{a\nonscript\hskip1pt b}$}\showbox1
\setbox1=\hbox{$a\nonscript\mskip1mu b^{a\nonscript\mskip1mu b}$}\showbox1
\setbox1=\hbox{$\nonscript a$}\showbox1
\setbox1=\hbox{$\scriptstyle\nonscript\kern1pt a$}\showbox1
\setbox1=\hbox{\nonscript}\showbox1
\nonscript\par
\setbox1=\hbox{$\scriptscriptstyle a\nonscript\;b\;c$}\showbox1
""")
P('mkern', r"""
\T{mkern}
\setbox1=\hbox{$a\mkern1mu b\mkern-1mu c\mkern18mu d$}\showbox1
\setbox1=\hbox{$\scriptstyle a\mkern1mu b$}\showbox1
\setbox1=\hbox{$\mkern 1pt$}
\setbox1=\hbox{$\mkern x$}
\setbox1=\hbox{\mkern1mu}\showbox1
\mkern1mu\par
\setbox1=\hbox{$\mkern 16384mu$}
\setbox1=\hbox{$\mkern\thinmuskip$}\showbox1
""")
P('mskip', r"""
\T{mskip}
\setbox1=\hbox{$a\mskip1mu b\mskip-1mu plus 1fil minus 1mu c\mskip\thinmuskip d$}\showbox1
\setbox1=\hbox to 100pt{$a\mskip 0mu plus 1fil b$}\showbox1
\setbox1=\hbox{$\scriptstyle a\mskip1mu b$}\showbox1
\setbox1=\hbox{$\mskip 1pt$}
\setbox1=\hbox{$\mskip x$}
\setbox1=\hbox{\mskip1mu}\showbox1
\mskip1mu\par
\setbox1=\hbox{$\mskip 16384mu$}
""")
P('over atop above overwithdelims atopwithdelims abovewithdelims', r"""
\T{fractions}
\def\d{\delimiter"4266308 \delimiter"5267309 }
\setbox1=\hbox{$a\over b$}\showbox1 \setbox1=\hbox{$a\atop b$}\showbox1 \setbox1=\hbox{$a\above1pt b$}\showbox1
\setbox1=\hbox{$a\overwithdelims() b$}\showbox1 \setbox1=\hbox{$a\atopwithdelims() b$}\showbox1 \setbox1=\hbox{$a\abovewithdelims()1pt b$}\showbox1
\setbox1=\hbox{$\displaystyle a\over b$}\showbox1
\setbox1=\hbox{$\scriptstyle a\over b$}\showbox1
\setbox1=\hbox{$\scriptscriptstyle a\over b$}\showbox1
\setbox1=\hbox{$a\above 0pt b$}\showbox1
\setbox1=\hbox{$a\above-1pt b$}\showbox1
\setbox1=\hbox{$a\over b\over c$}\showbox1
\setbox1=\hbox{$a\over b\atop c$}\showbox1
\setbox1=\hbox{$\over$}\showbox1
\setbox1=\hbox{$a\over$}\showbox1
\setbox1=\hbox{$\over b$}\showbox1
\setbox1=\hbox{$a\overwithdelims xy b$}
\setbox1=\hbox{$a\above x b$}
\setbox1=\hbox{$\left(a\over b\right)$}\showbox1
\setbox1=\hbox{$\mathchoice{a\over b}{a\atop b}{}{}$}\showbox1
\setbox1=\hbox{$\hbox{a}\over\vbox{\hbox{b}}$}\showbox1
\setbox1=\hbox{\over}\showbox1
\over\par
\atop\par
\above1pt\par
\overwithdelims()\par
\atopwithdelims()\par
\abovewithdelims()1pt\par
\setbox1=\hbox{$a^{b\over c}$}\showbox1
$$a\over b$$
""")
P('left right', r"""
\T{left right}
\setbox1=\hbox{$\left(a\right)$}\showbox1
\setbox1=\hbox{$\left(a\over b\right)$}\showbox1
\setbox1=\hbox{$\left.a\right|$}\showbox1
\setbox1=\hbox{$\left(\left[a\right]\right)$}\showbox1
\setbox1=\hbox{$\left(\vcenter{\hrule height 100pt}\right)$}\showbox1
\setbox1=\hbox{$\left(\hbox{a}^2\right)^2_1$}\showbox1
\setbox1=\hbox{$\scriptstyle\left(a\right)$}\showbox1
\setbox1=\hbox{\delimiterfactor=500 \delimitershortfall=0pt $\left(\vrule height 50pt\right)$}\showbox1
\setbox1=\hbox{\nulldelimiterspace=5pt $\left.a\right.$}\showbox1
\setbox1=\hbox{$\left a\right)$}
\setbox1=\hbox{$\left(a$}
\setbox1=\hbox{$a\right)$}
\setbox1=\hbox{$\left(a}$}
\setbox1=\hbox{$\left(a\right$}
\setbox1=\hbox{\left(a\right)}\showbox1
\left(\par
\right)\par
\setbox1=\hbox{$\left(\mathop{\rm a}\limits_1^2\right)$}\showbox1
""")
P('middle', r"""
\T{middle}
\setbox1=\hbox{$\left(a\middle|b\right)$}\showbox1
\setbox1=\hbox{$\left(a\middle|b\middle/c\middle.d\right)$}\showbox1
\setbox1=\hbox{$\left(\vrule height 50pt\middle|\right)$}\showbox1
\setbox1=\hbox{$\left(a\over b\middle|c\right)$}\showbox1
\setbox1=\hbox{$\left(a\middle|b\over c\right)$}\showbox1
\setbox1=\hbox{$\middle|$}
\setbox1=\hbox{$a\middle|b$}
\setbox1=\hbox{$\left(\middle x\right)$}
\setbox1=\hbox{\middle|}\showbox1
\middle|\par
\setbox1=\hbox{$\left(\left[\middle|\right]\middle|\right)$}\showbox1
""")
P('eqno leqno', r"""
\T{eqno leqno}
$$a\eqno(1)$$
$$a\leqno(1)$$
$$\eqno(1)$$
$$\leqno(1)$$
$$\hbox to \hsize{}\eqno(1)$$
$$\hbox to \hsize{}\leqno(1)$$
$$\hbox to 150pt{}\eqno(1)$$
$$\hbox to 150pt{}\leqno(1)$$
$$a\eqno\eqno$$
$$a\eqno(1)\leqno(2)$$
$$a\eqno$$
\setbox1=\hbox{$a\eqno(1)$}\showbox1
\eqno\par
\leqno\par
\noindent$a\eqno b$\par
$$\halign{#\cr a\cr}\eqno(1)$$
\setbox1=\vbox{\noindent a$$b\eqno(2)$$c\par}\showbox1
\abovedisplayskip=1pt \belowdisplayskip=2pt \abovedisplayshortskip=3pt \belowdisplayshortskip=4pt
\setbox1=\vbox{\noindent x$$y\eqno(3)$$\par}\showbox1
\setbox1=\vbox{\noindent\hbox to 180pt{}$$y$$\par}\showbox1
""")
P('vcenter', r"""
\T{vcenter}
\setbox1=\hbox{$\vcenter{\hbox{a}\hbox{b}}$}\showbox1
\setbox1=\hbox{$\vcenter to 50pt{\vfil\hbox{a}}$}\showbox1
\setbox1=\hbox{$\vcenter spread 5pt{\vfil\hbox{a}}$}\showbox1
\setbox1=\hbox{\vcenter{}}\showbox1
\vcenter{}\par
\setbox1=\hbox{$\vcenter x$}
\setbox1=\hbox{$\vcenter to x{}$}
\everyvbox={\V{ev}}\setbox1=\hbox{$\vcenter{}$}\everyvbox={}
\setbox1=\hbox{$\scriptstyle\vcenter{\hrule height 3pt}$}\showbox1
""")
P('textfont scriptfont scriptscriptfont', r"""
\T{math fonts}
\V{\the\textfont0 \the\scriptfont0 \the\scriptscriptfont0}
\showthe\textfont1 \showthe\scriptfont1 \showthe\scriptscriptfont1
\begingroup\textfont1=\tenrm \setbox1=\hbox{$\mathchar"0141$}\showbox1 \showthe\textfont1\endgroup\showthe\textfont1
\textfont16=\tenrm
\textfont-1=\tenrm
\textfont0=x
\textfont0=\relax
\scriptfont0=\tenbf \scriptscriptfont0=\tenit \setbox1=\hbox{$a^{\mathchar"0141^{\mathchar"0141}}$}\showbox1
\scriptfont0=\sevenrm \scriptscriptfont0=\fiverm
\begingroup\textfont2=\nullfont \setbox1=\hbox{$a^2$}\endgroup
\begingroup\textfont3=\nullfont \setbox1=\hbox{$\sum$}\endgroup
\begingroup\scriptfont2=\tenrm \setbox1=\hbox{$a^2$}\endgroup
\begingroup\font\small=cmr10 \textfont2=\small \setbox1=\hbox{$a_1^2$}\endgroup
\setbox1=\hbox{$\global\textfont5=\tenbf \fam=5 a\mathchar"7141$}\showbox1 \fam=-1
\setbox1=\hbox{$\fam=15 \mathchar"7141$}\showbox1 \fam=-1
\setbox1=\hbox{$\mathchar"0F41$}\showbox1
\textfont0=\tenrm
""")
# ---------------------------------------------------------------------------------------------- alignment
P('halign', r"""
\T{halign}
\halign{#\cr a\cr}
\halign to 100pt{\hfil#\tabskip 0pt plus 1fil&&#\hfil\cr a&b&c\cr d\cr e&f\cr}
\halign spread 10pt{#\tabskip 1pt plus 1fil&#\cr a&b\cr}
\setbox1=\vbox{\halign{#&#\cr a&b\cr\noalign{\hrule}c&d\cr}}\showbox1
\setbox1=\vbox{\halign{#&#\cr\omit a&b\cr c\span d&e\cr\omit\span\omit f\cr}}\showbox1
\setbox1=\vbox{\halign{#\tabskip 3pt&#\tabskip 5pt&#\cr a&b&c\cr}}\showbox1
\setbox1=\hbox{\halign{#\cr}}\showbox1
\setbox1=\hbox{$\halign{#\cr}$}\showbox1
\setbox1=\vbox{\halign{#\cr}}\showbox1
\setbox1=\vbox{\halign{a#b\cr c\cr}}\showbox1
\setbox1=\vbox{\halign{&#\cr a&b&c\cr}}\showbox1
\setbox1=\vbox{\halign{#&&#&#\cr a&b&c&d&e\cr}}\showbox1
\setbox1=\vbox{\halign{##\cr}}
\setbox1=\vbox{\halign{x\cr y\cr}}
\setbox1=\vbox{\halign{#\cr a&b\cr}}
\setbox1=\vbox{\halign{#&\cr a&b\cr}}
\setbox1=\vbox{\halign{#\cr a}}
\setbox1=\vbox{\halign{#\cr{a\cr}\cr}}
\setbox1=\vbox{\halign{#\cr\vrule\cr}}\showbox1
\halign x\par
\halign to x{}
\everycr={\noalign{\V{everycr}}}\setbox1=\vbox{\halign{#\cr a\cr b\cr}}\everycr={}
\setbox1=\vbox{\baselineskip=20pt \halign{#\cr a\cr b\cr}}\showbox1
\setbox1=\vbox{\tabskip=5pt \halign{#\cr a\cr}}\showbox1
$$\halign{#\cr a\cr}$$
$$\halign to 100pt{#\tabskip 0pt plus 1fil\cr a\cr}$$
$$\halign{#\cr a\cr}x$$
$$x\halign{#\cr a\cr}$$
\setbox1=\vbox{\halign{#\cr\noalign{\vskip 1pt\penalty 5 \kern 1pt}a\cr}}\showbox1
\setbox1=\vbox{\halign{\def\x{y}#\cr\show\x a\cr}}
\setbox1=\vbox{\halign{\global\def\x{y}#\cr\show\x a\cr}}\show\x
\setbox1=\vbox{\halign{\span\relax#\cr a\cr}}\showbox1
\def\p{\tabskip1pt}\setbox1=\vbox{\halign{\p#\span\p\cr a\cr}}\showbox1
""")
P('valign', r"""
\T{valign}
\setbox1=\hbox{\valign{#\vfil\cr\hbox{a}\cr\hbox{b}\cr}}\showbox1
\setbox1=\hbox{\valign to 50pt{#\tabskip 0pt plus 1fil&#\cr\hbox{a}&\hbox{b}\cr}}\showbox1
\setbox1=\hbox{\valign spread 5pt{#&#\cr\hbox{a}&\hbox{b}\cr\noalign{\vrule}\hbox{c}&\hbox{d}\cr}}\showbox1
\setbox1=\hbox{\valign{#\cr\omit\hbox{x}\cr}}\showbox1
\noindent\valign{#\cr\hbox{a}\cr}\par
\valign{#\cr\hbox{a}\cr}
\setbox1=\vbox{\valign{#\cr\hbox{a}\cr}}\showbox1
\setbox1=\hbox{$\valign{#\cr}$}\showbox1
$$\valign{#\cr}$$
\setbox1=\hbox{\valign{#\cr a\cr}}\showbox1
\setbox1=\hbox{\valign x}
""")
P('noalign', r"""
\T{noalign}
\setbox1=\vbox{\halign{#\cr\noalign{\hrule}a\cr\noalign{\kern1pt}\noalign{\penalty5}b\cr\noalign{}}}\showbox1
\noalign{}
\setbox1=\hbox{\noalign{}}
\setbox1=\vbox{\halign{#\cr a\noalign{}\cr}}
\setbox1=\vbox{\halign{#&#\cr a&\noalign{}\cr}}
\setbox1=\vbox{\halign{#\cr\noalign x a\cr}}
\setbox1=\vbox{\halign{#\cr\noalign{\hbox{}}\cr}}\showbox1
\setbox1=\vbox{\halign{#\cr\noalign{\def\x{y}}a\cr\show\x}}
\setbox1=\hbox{\valign{#\cr\noalign{\vrule}\hbox{}\cr}}\showbox1
\setbox1=\vbox{\halign{#\cr\noalign{a\par}\cr}}\showbox1
""")
P('omit', r"""
\T{omit}
\setbox1=\vbox{\halign{x#y&u#v\cr\omit a&\omit b\cr c&d\cr}}\showbox1
\omit
\setbox1=\hbox{\omit}
\setbox1=\vbox{\halign{#\cr a\omit\cr}}
\setbox1=\vbox{\halign{#&#\cr\omit\omit a&b\cr}}
\setbox1=\vbox{\halign{#\cr\omit\cr}}\showbox1
\setbox1=\vbox{\halign{#&#\cr\omit\span\omit a\cr}}\showbox1
\def\o{\omit}\setbox1=\vbox{\halign{x#\cr\o a\cr}}\showbox1
""")
P('span', r"""
\T{span}
\setbox1=\vbox{\halign{#&#&#\cr a\span b\span c\cr d\span e&f\cr}}\showbox1
\span
\setbox1=\hbox{\span}
\setbox1=\vbox{\halign{#&#\cr\omit\span\omit\span\omit x\cr}}\showbox1
\def\t{x#y}\setbox1=\vbox{\halign{\span\t\cr a\cr}}\showbox1
\def\e{\undefinedthing}\setbox1=\vbox{\halign{\span\e#\cr a\cr}}
\def\m{a}\setbox1=\vbox{\halign{#\span\m\cr b\cr}}\showbox1
\setbox1=\vbox{\tabskip=2pt\halign{#\tabskip=1pt&#\cr a\span b\cr}}\showbox1
""")
P('cr crcr', r"""
\T{cr crcr}
\cr
\crcr
\setbox1=\hbox{\cr}\setbox1=\hbox{\crcr}
\setbox1=\vbox{\halign{#\cr a\crcr}}\showbox1
\setbox1=\vbox{\halign{#\crcr a\cr\crcr\crcr}}\showbox1
\setbox1=\vbox{\halign{#\cr a\cr\crcr\noalign{}\crcr}}\showbox1
\setbox1=\vbox{\halign{#\cr\crcr\cr}}\showbox1
\setbox1=\vbox{\halign{#\cr\cr\cr}}\showbox1
\setbox1=\vbox{\halign{#&#\cr a\cr b&c\cr}}\showbox1
\setbox1=\vbox{\halign{#\cr{\cr}\cr}}
\everycr={\V{everycr}}\setbox1=\vbox{\halign{#\cr a\cr\crcr}}\everycr={}
\show\cr\show\crcr
\setbox1=\hbox{$\cr$}\setbox1=\hbox{$\crcr$}
""")
# ---------------------------------------------------------------------------------------------- arithmetic
P('advance multiply divide', r"""
\T{arithmetic}
\count1=5 \advance\count1 by 3 \showthe\count1 \advance\count1 3 \showthe\count1
\multiply\count1 by 3 \showthe\count1 \multiply\count1 3 \showthe\count1
\divide\count1 by 3 \showthe\count1 \divide\count1 3 \showthe\count1
\count1=-7 \divide\count1 by 2 \showthe\count1 \count1=7 \divide\count1 by-2 \showthe\count1
\divide\count1 by 0
\count1=2147483647 \advance\count1 by 1 \showthe\count1
\count1=2147483647 \multiply\count1 by 2 \showthe\count1
\dimen1=1pt \advance\dimen1 by 1pt \showthe\dimen1 \multiply\dimen1 by 3 \showthe\dimen1 \divide\dimen1 by 3 \showthe\dimen1
\dimen1=16383pt \advance\dimen1 by 16383pt \showthe\dimen1
\dimen1=16383pt \multiply\dimen1 by 16383 \showthe\dimen1
\skip1=1pt plus 2pt minus 3pt \advance\skip1 by 1pt plus 1fil \showthe\skip1 \multiply\skip1 by 2 \showthe\skip1 \divide\skip1 by 2 \showthe\skip1
\muskip1=1mu plus 2mu \advance\muskip1 by 1mu plus 1fil \showthe\muskip1 \multiply\muskip1 by 2 \showthe\muskip1 \divide\muskip1 by 2 \showthe\muskip1
\tolerance=100 \advance\tolerance by 1 \showthe\tolerance \multiply\tolerance by 2 \showthe\tolerance \divide\tolerance by 2 \showthe\tolerance
\parindent=1pt \advance\parindent by 1pt \showthe\parindent \multiply\parindent by 2 \showthe\parindent \divide\parindent by 2 \showthe\parindent
\baselineskip=12pt \advance\baselineskip by 1pt \showthe\baselineskip \multiply\baselineskip by 2 \showthe\baselineskip \divide\baselineskip by 2 \showthe\baselineskip
\baselineskip=12pt
\countdef\c=7 \c=1 \advance\c by 1 \showthe\c
\advance\relax by 1
\advance x
\multiply\relax by 1
\multiply x
\divide\relax by 1
\divide x
\advance\count1 by x
\multiply\count1 by x
\divide\count1 by x
\advance\wd0 by 1pt \advance\fontdimen1\tenrm by 0pt \showthe\fontdimen1\tenrm
\advance\hyphenchar\tenrm by 0 \multiply\skewchar\tenrm by 1
\advance\spacefactor by 1
\advance\prevdepth by 1pt \advance\pagegoal by 0pt
\global\advance\count1 by 1 \global\multiply\count1 by 1 \global\divide\count1 by 1
\begingroup\advance\count1 by 1 \endgroup\showthe\count1
\advance\toks0 by 1
\advance\catcode`a by 0
\advance\parshape by 1
""")
# ---------------------------------------------------------------------------------------------- fonts
P('font', r"""
\T{font}
\font\a=cmr10 \show\a \V{\meaning\a \fontname\a}\setbox1=\hbox{\a x}\showbox1
\font\a=cmr10 at 12pt \show\a\V{\fontname\a}\setbox1=\hbox{\a x}\showbox1
\font\a=cmr10 scaled 1200 \V{\fontname\a}
\font\a=cmr10 scaled 1000 \V{\fontname\a}
\font\a=cmr10 at 10pt \V{\fontname\a}\ifx\a\tenrm\V{same as tenrm}\fi
\font\a=cmr10 at 0pt
\font\a=cmr10 at -1pt
\font\a=cmr10 at 2048pt
\font\a=cmr10 at 2047.99999pt \V{\fontname\a}
\font\a=cmr10 scaled 0
\font\a=cmr10 scaled -1
\font\a=cmr10 scaled 32769
\font\a=cmr10 scaled 32768 \V{\fontname\a}
\font\a=nosuchfontname
\show\a\V{\fontname\a}
\font\a={cmr10}\V{\fontname\a}
\font\a="cmr10" \V{\fontname\a}
\font\a=cmr10.tfm \V{\fontname\a}
\font\a=cmr10 at x
\font\a=cmr10 scaled x
\font\a= \par
\font a=cmr10
\font\relax=cmr10 \show\relax\let\relax=\undefinedthing
\begingroup\font\b=cmbx10 \show\b\endgroup\show\b
\global\font\b=cmbx10 \show\b
\begingroup\global\font\c=cmti10 \endgroup\show\c
\font\c=cmr10\b x\par
\showthe\font\V{\the\font}\V{\meaning\font}\V{\fontname\font}
\setbox1=\hbox{\nullfont xyz}\showbox1
\font\z=cmr5 \font\y=cmr5 at 5pt \ifx\z\y\V{shared}\else\V{distinct}\fi
\setbox1=\hbox{\font\a=cmr7 \a text}\showbox1
\font\x=cmr10 \fontdimen8\x=1pt
\font\x=cmr10 at 11pt \fontdimen100\x=1pt \showthe\fontdimen100\x
""")
P('nullfont', r"""
\T{nullfont}
\show\nullfont\V{\meaning\nullfont \fontname\nullfont}
\setbox1=\hbox{\nullfont abc \char65 \accent65 x}\showbox1
\V{\the\fontdimen1\nullfont}
\fontdimen1\nullfont=1pt
\showthe\hyphenchar\nullfont\showthe\skewchar\nullfont
\hyphenchar\nullfont=5 \showthe\hyphenchar\nullfont
\skewchar\nullfont=5 \showthe\skewchar\nullfont
\begingroup\nullfont\showthe\font\endgroup
\nullfont\setbox1=\vbox{\noindent a\par}\showbox1\tenrm
\textfont0=\nullfont \textfont0=\tenrm
\ifx\font\nullfont\V{is null}\else\V{not null}\fi
\iffontchar\nullfont65 \V{has}\else\V{has not}\fi
""")
P('fontdimen', r"""
\T{fontdimen}
\showthe\fontdimen1\tenrm\showthe\fontdimen2\tenrm\showthe\fontdimen7\tenrm
\showthe\fontdimen8\tenrm
\showthe\fontdimen0\tenrm
\showthe\fontdimen-1\tenrm
\fontdimen2\tenrm=5pt \setbox1=\hbox{a b}\showbox1
\begingroup\fontdimen2\tenrm=6pt \endgroup\showthe\fontdimen2\tenrm
\fontdimen2\tenrm=3.33333pt
\fontdimen1\tenmi=0pt \fontdimen22\tensy=3pt \showthe\fontdimen22\tensy
\font\f=cmr10 at 13pt \fontdimen8\f=1pt \fontdimen9\f=1pt \showthe\fontdimen9\f
\fontdimen100\f=1pt \showthe\fontdimen100\f \showthe\fontdimen99\f
\font\g=cmr10 at 14pt \setbox1=\hbox{\g a}\fontdimen8\g=1pt
\fontdimen x\tenrm=1pt
\fontdimen1 x=1pt
\fontdimen1\relax=1pt
\advance\fontdimen2\tenrm by 1pt \showthe\fontdimen2\tenrm \fontdimen2\tenrm=3.33333pt
\fontdimen1\font=0pt
\fontdimen2\tenrm=16384pt
\V{\the\fontdimen2\font}
\multiply\fontdimen3\tenrm by 1
""")
P('hyphenchar skewchar', r"""
\T{hyphenchar skewchar}
\showthe\hyphenchar\tenrm\showthe\skewchar\tenrm\showthe\skewchar\teni\showthe\skewchar\tensy
\hyphenchar\tenrm=-1 \showthe\hyphenchar\tenrm \setbox1=\vbox{\hsize=20pt\noindent typesetting\par}\showbox1
\hyphenchar\tenrm=45 \setbox1=\vbox{\hsize=20pt\noindent typesetting\par}\showbox1
\hyphenchar\tenrm=256 \showthe\hyphenchar\tenrm \setbox1=\vbox{\hsize=20pt\noindent typesetting\par}\showbox1
\hyphenchar\tenrm=2147483647 \hyphenchar\tenrm=-2147483647 \hyphenchar\tenrm=45
\skewchar\teni=-1 \showthe\skewchar\teni \setbox1=\hbox{$\mathaccent"7014 a$}\showbox1
\skewchar\teni=127 \skewchar\teni=256 \showthe\skewchar\teni \skewchar\teni=127
\begingroup\hyphenchar\tenrm=5 \endgroup\showthe\hyphenchar\tenrm \hyphenchar\tenrm=45
\begingroup\skewchar\teni=5 \endgroup\showthe\skewchar\teni \skewchar\teni=127
\hyphenchar\relax=1
\skewchar x=1
\hyphenchar\tenrm=x
\defaulthyphenchar=64 \font\f=cmr10 at 15pt \showthe\hyphenchar\f \defaulthyphenchar=45
\defaultskewchar=64 \font\f=cmr10 at 16pt \showthe\skewchar\f \defaultskewchar=-1
\advance\hyphenchar\tenrm by 1 \advance\hyphenchar\tenrm by -1
\showthe\hyphenchar\font
\hyphenchar\font=45
""")
P('fontname', r"""
\T{fontname}
\V{\fontname\tenrm}\V{\fontname\nullfont}\V{\fontname\font}
\font\f=cmr10 at 12.5pt \V{\fontname\f}
\font\f=cmr10 scaled 1100 \V{\fontname\f}
\font\f=cmr5 scaled 2000 \V{\fontname\f}
\V{\fontname x}
\V{\fontname\relax}
\edef\a{\fontname\tenrm}\show\a
\fontname\tenrm\par
\expandafter\show\fontname\tenrm\relax
""")
# ---------------------------------------------------------------------------------------------- output of messages and diagnostics
P('message', r"""
\T{message}
\message{a}\message{b}\message{c
d}\message{}\message{\relax}\message{\the\count0}
\def\a{A}\message{\a\noexpand\a}\message{\undefinedthing}
\message{#}\message{##}
\message{xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx}
\message{short}\message{xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx}\message{yy}
\newlinechar=`| \message{one|two|}\message{three}\newlinechar=-1
\message x
\message{}}
\setbox1=\hbox{\message{in box}}
\message{^^80^^ff^^M^^J}
\message{\string\a}
""")
P('errmessage', r"""
\T{errmessage}
\errmessage{first}
\errmessage{}
\errmessage{\a\b}
\def\a{A}\errmessage{\a\noexpand\a}
\errhelp={help line one}\errmessage{with help}
\errhelp={}\errmessage{no help}
\newlinechar=`| \errhelp={line1|line2|line3}\errmessage{multi|line|error}\newlinechar=-1 \errhelp={}
\errorcontextlines=-1 \errmessage{no context}\errorcontextlines=100
\def\b{\c}\def\c{\errmessage{deep}}\b
\errmessage x
\setbox1=\hbox{\errmessage{in box}}
$\errmessage{in math}$
\errmessage{xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx}
""")
P('show', r"""
\T{show}
\show\show\show a\show{\show}\show$\show&\show#\show^\show_\show~\show 1\show\undefinedthing
\def\a{}\show\a \def\a#1#2.{x#1}\show\a \long\def\a{}\show\a \outer\def\a{}\show\a \protected\def\a{}\show\a
\let\a= \space\show\a
\show\hbox\show\tolerance\show\baselineskip\show\tenrm\show\nullfont
\chardef\b=5 \show\b \mathchardef\b=5 \show\b \countdef\b=5 \show\b
\show
\relax
\setbox1=\hbox{\show\a}
\errorcontextlines=-1 \show\a\errorcontextlines=100
\showboxbreadth=-1 \show\a\showboxbreadth=300
\catcode`\~=13 \def~{t}\show~
\show\par\show\ \show\-\show\/
\show\endtemplate
\expandafter\show\expandafter\a\relax
""")
P('showthe', r"""
\T{showthe}
\showthe\count0 \showthe\dimen0 \showthe\skip0 \showthe\muskip0 \showthe\toks0
\showthe\tolerance\showthe\baselineskip\showthe\thinmuskip\showthe\everypar
\showthe\font\showthe\tenrm\showthe\textfont0 \showthe\hyphenchar\tenrm\showthe\fontdimen1\tenrm
\showthe\catcode`a\showthe\wd0 \showthe\parshape\showthe\inputlineno\showthe\badness
\showthe\relax
\showthe x
\showthe\undefinedthing
\showthe\par
\showthe
\setbox1=\hbox{\showthe\count0}
\toks0={a#b\c}\showthe\toks0
\showthe\numexpr 1+1\relax
\showthe\spacefactor
\showthe\prevdepth
\showthe\pagegoal
\setbox1=\hbox{\showthe\spacefactor \showthe\prevdepth}
\setbox1=\vbox{\showthe\spacefactor \showthe\prevdepth}
""")
P('showbox', r"""
\T{showbox}
\showbox0 \showbox255 \showbox32767 \showbox32768 \showbox-1 \showbox x
\setbox1=\hbox{a\kern1pt\vrule\hskip1pt plus1fil\penalty5 \discretionary{a}{b}{c}\mark{m}\insert100{}$a^2$\special{s}\vadjust{}\write-1{w}\openout1=f \closeout1 \leaders\hrule\hfil \hbox{}\vbox{}\vtop{}\accent127 e\/}\showbox1
\showboxbreadth=0 \showbox1 \showboxbreadth=1 \showbox1 \showboxbreadth=-1 \showbox1 \showboxbreadth=300
\showboxdepth=0 \showbox1 \showboxdepth=-1 \showbox1 \showboxdepth=30
\setbox1=\hbox to 10pt{\hskip 0pt plus 1fil}\showbox1
\setbox1=\hbox to 10pt{\hskip 0pt plus 3fill}\showbox1
\setbox1=\hbox to 10pt{\hskip 0pt plus 7filll}\showbox1
\setbox1=\hbox to 10pt{xxxxx\hskip 0pt minus 1fil}\showbox1
\setbox1=\hbox to 10pt{x\hskip 0pt plus 3pt}\showbox1
\setbox1=\hbox{\raise1pt\hbox{}\lower1pt\vbox{}}\showbox1
\setbox1=\vbox{\moveleft1pt\hbox{}}\showbox1
\setbox1=\hbox{\pdfliteral{x}\pdfsave\pdfrestore}\showbox1
\setbox1=\hbox{\mathsurround=1pt $a$}\showbox1
\setbox1=\hbox{\char128 \char255}\showbox1
\setbox1=\vbox{\showbox1}
\setbox1=\hbox{}\wd1=1pt \ht1=2pt \dp1=3pt \showbox1
""")
P('showlists', r"""
\T{showlists}
\showlists
\hbox{a}\penalty5 \showlists
\noindent a\showlists\par
\setbox1=\hbox{a\setbox2=\vbox{b\setbox3=\hbox{$c\showlists$}\par}}
$$\showlists a\eqno\showlists b$$
\setbox1=\hbox{$\left(\showlists a\over\showlists b\right)$}
\setbox1=\hbox{$\mathchoice{\showlists}{\showlists}{\showlists}{\showlists}$}
\setbox1=\vbox{\halign{#\showlists\cr a\cr\noalign{\showlists}}}
\insert100{\showlists}\noindent\vadjust{\showlists}\par
\output={\showlists\setbox0=\box255 \deadcycles=0 }
\hbox{x}\penalty-10000
\showlists
\setbox1=\hbox{\spacefactor=5 \showlists}
\setbox1=\vbox{\prevdepth=5pt \showlists}
\prevgraf=7 \showlists\prevgraf=0
\pagegoal=5pt \showlists
""")
P('pausing', r"""
\T{pausing}
\pausing=1
\V{paused line}
\pausing=0
\showthe\pausing
""")
P('batchmode nonstopmode scrollmode', r"""
\T{interaction commands}
\showthe\interactionmode
\batchmode \showthe\interactionmode \undefinedA
\nonstopmode \showthe\interactionmode \undefinedB
\scrollmode \showthe\interactionmode \undefinedC
\batchmode \showthe\interactionmode \undefinedD
\begingroup\scrollmode\endgroup\showthe\interactionmode
\global\batchmode \showthe\interactionmode
\batchmode
""")
P('special', r"""
\T{special}
\special{hello}\setbox1=\hbox{\special{in box}}\showbox1
\special{}\def\a{A}\setbox1=\hbox{\special{\a\noexpand\a#}}\showbox1
\setbox1=\hbox{$\special{in math}$}\showbox1
\special x
\special{}}
\shipout\hbox{\special{at shipout \the\count0}}
\setbox1=\vbox{\special{v}}\showbox1
\pdfoutput=1
""")
# ---------------------------------------------------------------------------------------------- reading and writing
P('input', r"""
\T{input}
\immediate\openout1=probeinput.tex
\immediate\write1{\string\V{inside input, line \string\the\string\inputlineno}}
\immediate\write1{\string\endinput}
\immediate\write1{\string\V{not reached}}
\immediate\closeout1
\input probeinput.tex
\input probeinput.tex \V{after space}
\input "probeinput.tex"
\input{probeinput.tex}
\V{\jobname}
\def\n{probeinput.tex}\input\n\space
\expandafter\input\expandafter p\expandafter r\expandafter o\expandafter b\expandafter e\expandafter i\expandafter n\expandafter p\expandafter u\expandafter t\expandafter.\expandafter t\expandafter e\expandafter x\space
\immediate\openout1=probeinput2.tex
\immediate\write1{\string\V{no newline at end}\string\relax}
\immediate\closeout1
\input probeinput2.tex
\everyeof={\V{everyeof ran}}\input probeinput2.tex \everyeof={}
\show\input
""")
P('endinput', r"""
\T{endinput}
\immediate\openout1=probeendinput.tex
\immediate\write1{\string\V{a}\string\endinput\string\V{same line}}
\immediate\write1{\string\V{not reached}}
\immediate\closeout1
\input probeendinput.tex
\immediate\openout1=probeendinput.tex
\immediate\write1{\string\def\string\m{\string\endinput}\string\m\string\V{after macro}}
\immediate\write1{\string\V{next line}}
\immediate\closeout1
\input probeendinput.tex
\scantokens{\V{before}\endinput\V{same line in scantokens}
\V{not reached}}
\show\endinput
\expandafter\show\csname endinput\endcsname
""")
P('openin closein', r"""
\T{openin closein}
\openin1=plain.tex \ifeof1 \V{plain eof}\else\V{plain open}\fi\closein1
\openin1=nosuchfile.xyz \ifeof1 \V{missing eof}\else\V{missing open}\fi\closein1
\openin15=plain.tex \ifeof15 \V{15 eof}\else\V{15 open}\fi\closein15
\openin16=plain.tex
\openin-1=plain.tex
\closein16
\closein-1
\openin x=plain.tex
\closein x
\closein2
\openin1=\jobname\relax \ifeof1 \V{job eof}\else\V{job open}\fi\closein1
\openin1=plain \ifeof1 \V{ext eof}\else\V{ext open}\fi
\openin1=plain \closein1
\setbox1=\hbox{\openin2=plain.tex \closein2}
\showbox1
\begingroup\openin3=plain.tex \endgroup\ifeof3 \V{closed by group}\else\V{open after group}\fi\closein3
\immediate\openin4=plain.tex
\closein4
""")
P('read', r"""
\T{read}
\immediate\openout1=proberead.tex
\immediate\write1{first line}
\immediate\write1{\string\def\string\x{y}}
\immediate\write1{{braces}}
\immediate\write1{\lb unbalanced}
\immediate\write1{continued\rb}
\immediate\write1{ }
\immediate\write1{last}
\immediate\closeout1
\openin1=proberead.tex \ifeof1 \V{not found}\fi
\read1 to\a\show\a\read1 to\a\show\a\read1 to\a\show\a\read1 to\a\show\a
\read1 to\a\show\a\read1 to\a\show\a\read1 to\a\show\a\ifeof1 \V{eof now}\else\V{not eof}\fi
\read1 to\a\show\a\ifeof1 \V{eof now}\else\V{not eof}\fi
\openin1=proberead.tex
\endlinechar=-1 \read1 to\a\show\a\endlinechar=13
\catcode`\f=12 \read1 to\a\show\a\catcode`\f=11
\global\read1 to\a\show\a
\begingroup\read1 to\a\show\a\endgroup\show\a
\long\read1 to\a
\closein1
\read1 x\a
\read x to\a
\read1 to a
\read1 to
""")
P('readline', r"""
\T{readline}
\immediate\openout1=probereadline.tex
\immediate\write1{first \string\line {with} $ \pc\space specials}
\immediate\write1{  leading and trailing   }
\immediate\write1{}
\immediate\write1{\lb unbalanced}
\immediate\closeout1
\openin1=probereadline.tex
\readline1 to\a\show\a\V{\meaning\a}
\readline1 to\a\show\a\V{\meaning\a}
\readline1 to\a\show\a\V{\meaning\a}
\readline1 to\a\show\a\V{\meaning\a}
\ifeof1 \V{eof}\else\V{not eof}\fi
\readline1 to\a\show\a\V{\meaning\a}\ifeof1 \V{eof}\else\V{not eof}\fi
\openin1=probereadline.tex
\endlinechar=`X \readline1 to\a\show\a\endlinechar=13
\endlinechar=-1 \readline1 to\a\show\a\endlinechar=13
\global\readline1 to\a\show\a
\closein1
\readline1 x\a
\readline x to\a
\readline1 to a
""")
P('openout closeout write immediate', r"""
\T{writes}
\immediate\openout1=probewrite.out
\immediate\write1{a line}
\immediate\write1{}
\immediate\write1{\relax\par\the\count0 ##}
\def\a{A}\immediate\write1{\a\noexpand\a\string\a}
\immediate\write1{xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx}
\newlinechar=`| \immediate\write1{one|two|three}\newlinechar=-1
\immediate\closeout1
\immediate\write1{after close goes to log}
\immediate\write16{to terminal and log}
\immediate\write-1{to log only}
\immediate\write17{to terminal and log}
\immediate\write15{not open}
\immediate\write 2147483647{big}
\immediate\write-2147483647{big negative}
\immediate\write 2147483648{too big}
\immediate\openout16=x
\immediate\openout-1=x
\immediate\closeout16
\immediate\closeout-1
\immediate\closeout5
\immediate\openout x=y
\immediate\write x{y}
\immediate\write1 x
\immediate\relax
\immediate x\par
\immediate\penalty5
\immediate\hbox{}
\setbox1=\hbox{\openout3=probedelayed.out\write3{delayed \the\count0}\closeout3}\showbox1
\shipout\box1
\openout4=probevertical.out
\write4{vertical}
\closeout4
\vfill\penalty-10000
\setbox1=\hbox{\write-1{\the\inputlineno}\write-1{\undefinedthing}\write-1{\noexpand\relax}}\shipout\box1
\setbox1=\hbox{\write-1{\errmessage{in write}}}\shipout\box1
\setbox1=\hbox{\write-1{\iffalse}}\shipout\box1
\setbox1=\hbox{$\openout5=m.out\write5{m}\closeout5$}\showbox1
\immediate\openout1=probewrite2.out \immediate\openout1=probewrite3.out \immediate\write1{second}
\immediate\closeout1
\global\immediate\write-1{prefix}
""")
P('setlanguage', r"""
\T{setlanguage}
\setbox1=\hbox{a\setlanguage5 b}\showbox1
\setbox1=\vbox{\language=7 \noindent a\setlanguage5 b\par}\showbox1
\setlanguage5
\setlanguage 256
\setlanguage -1
\setlanguage x
\setbox1=\hbox{$\setlanguage1$}\showbox1
\setbox1=\vbox{\hsize=20pt\noindent typesetting\setlanguage1 typesetting\par}\showbox1
""")
P('jobname', r"""
\T{jobname}
\V{\jobname}\edef\a{\jobname}\show\a
\expandafter\show\jobname\relax
\jobname\par
\def\b{doc}\edef\c{\jobname}\ifx\b\c\V{same}\else\V{catcodes differ}\fi
\show\jobname
""")
P('inputlineno', r"""
\T{inputlineno}
\showthe\inputlineno\V{\the\inputlineno}\V{\number\inputlineno}
\def\a{\the\inputlineno}\V{\a}
\V{\the\inputlineno
}
\count1=\inputlineno\showthe\count1
\inputlineno=5
\advance\inputlineno by 1
\scantokens{\V{\the\inputlineno}}\V{\the\inputlineno}
""")
P('badness', r"""
\T{badness}
\setbox1=\hbox to 10pt{\hskip0pt plus 10pt}\showthe\badness
\setbox1=\hbox to 10pt{\hskip0pt plus 5pt}\showthe\badness
\setbox1=\hbox to 10pt{\hskip0pt plus 1fil}\showthe\badness
\setbox1=\hbox to 10pt{\hskip20pt minus 10pt}\showthe\badness
\setbox1=\hbox to 10pt{\hskip20pt minus 5pt}\showthe\badness
\setbox1=\hbox to 10pt{\hskip10pt}\showthe\badness
\setbox1=\hbox to 10pt{}\showthe\badness
\setbox1=\hbox{}\showthe\badness
\setbox1=\vbox to 10pt{\vskip0pt plus 3pt}\showthe\badness
\setbox1=\vbox to 10pt{\vskip20pt minus 3pt}\showthe\badness
\badness=1
\advance\badness by 1
""")
P('lastpenalty lastkern lastskip', r"""
\T{last items}
\setbox1=\hbox{\penalty5 \showthe\lastpenalty\kern5pt\showthe\lastkern\hskip5pt plus 1fil\showthe\lastskip
  \showthe\lastpenalty\showthe\lastkern
  \unskip\showthe\lastkern\showthe\lastskip\unkern\showthe\lastpenalty\unpenalty\showthe\lastpenalty}
\setbox1=\vbox{\penalty5 \showthe\lastpenalty\kern5pt\showthe\lastkern\vskip5pt plus 1fil\showthe\lastskip}
\setbox1=\hbox{$\penalty5 \showthe\lastpenalty\kern5pt\showthe\lastkern\hskip5pt\showthe\lastskip\mskip5mu\showthe\lastskip$}
\showthe\lastpenalty\showthe\lastkern\showthe\lastskip
\penalty5 \showthe\lastpenalty
\hbox{}\kern1pt\showthe\lastkern\showthe\lastskip\vskip1pt\showthe\lastskip\showthe\lastkern
\lastpenalty=1
\lastkern=1pt
\lastskip=1pt
\setbox1=\hbox{\penalty0 \showthe\lastpenalty a\showthe\lastpenalty\showthe\lastkern\showthe\lastskip}
""")
P('spacefactor', r"""
\T{spacefactor}
\showthe\spacefactor
\setbox1=\hbox{\showthe\spacefactor\spacefactor=2000 \showthe\spacefactor a\showthe\spacefactor.\showthe\spacefactor A\showthe\spacefactor)\showthe\spacefactor}
\setbox1=\hbox{\spacefactor=0 }
\setbox1=\hbox{\spacefactor=-1 }
\setbox1=\hbox{\spacefactor=32767 \showthe\spacefactor \spacefactor=32768 \showthe\spacefactor}
\setbox1=\hbox{\spacefactor=2000 \kern1pt\showthe\spacefactor\hbox{}\showthe\spacefactor$a$\showthe\spacefactor}
\setbox1=\hbox{$\showthe\spacefactor\spacefactor=1 $}
\spacefactor=1000
\noindent a\showthe\spacefactor\par
\setbox1=\hbox{\spacefactor=3000 \ a}\showbox1
\setbox1=\hbox{\spacefactor=3000 a \spacefactor=3000 b}\showbox1
\advance\spacefactor by 1
\setbox1=\hbox{\advance\spacefactor by 1 \showthe\spacefactor \multiply\spacefactor by 2 \showthe\spacefactor \divide\spacefactor by 3 \showthe\spacefactor \global\spacefactor=5 \showthe\spacefactor}
""")
P('prevdepth', r"""
\T{prevdepth}
\showthe\prevdepth
\setbox1=\vbox{\showthe\prevdepth\prevdepth=5pt \showthe\prevdepth\hbox{}\showthe\prevdepth\hrule\showthe\prevdepth\kern1pt\showthe\prevdepth}\showbox1
\setbox1=\vbox{\prevdepth=-1000pt \hbox{}\showbox1}
\setbox1=\vbox{\prevdepth=-16383pt \hbox{}\hbox{}\prevdepth=-1000pt \hbox{}}\showbox1
\setbox1=\vbox{\hrule depth 3pt \showthe\prevdepth\hbox{}}\showbox1
\setbox1=\hbox{\showthe\prevdepth\prevdepth=1pt }
\setbox1=\hbox{$\showthe\prevdepth\prevdepth=1pt $}
\prevdepth=0pt \showthe\prevdepth \prevdepth=-1000pt
\advance\prevdepth by 1pt \showthe\prevdepth \prevdepth=-1000pt
\setbox1=\vbox{\global\prevdepth=3pt \showthe\prevdepth}
\prevdepth=x
""")
P('prevgraf', r"""
\T{prevgraf}
\showthe\prevgraf
\prevgraf=5 \showthe\prevgraf
\prevgraf=-1
\prevgraf=2147483647 \showthe\prevgraf
\prevgraf=0
\setbox1=\vbox{\hsize=50pt \noindent one two three four five six seven\par\showthe\prevgraf
  $$a$$\showthe\prevgraf
  \noindent a\showthe\prevgraf\par\showthe\prevgraf}
\setbox1=\hbox{\showthe\prevgraf\prevgraf=3 \showthe\prevgraf}
\noindent\prevgraf=7 \showthe\prevgraf\par\showthe\prevgraf
\begingroup\prevgraf=9 \endgroup\showthe\prevgraf
\advance\prevgraf by 1 \showthe\prevgraf
\prevgraf=0
\setbox1=\vbox{\hangindent=5pt \hangafter=2 \prevgraf=1 \noindent x\par\showthe\prevgraf}\showbox1
""")
P('parshape', r"""
\T{parshape}
\showthe\parshape
\parshape=3 0pt 100pt 10pt 90pt 20pt 80pt \showthe\parshape\V{\the\parshape}
\setbox1=\vbox{\parshape=3 0pt 100pt 10pt 90pt 20pt 80pt \noindent one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen\par\showthe\parshape}\showbox1
\setbox1=\vbox{\parshape=1 5pt 50pt one two three four five six\par}\showbox1
\setbox1=\vbox{\parshape=2 0pt 30pt 16383pt 1pt one two three four five six\par}\showbox1
\parshape=0 \showthe\parshape
\parshape=-1 \showthe\parshape
\parshape=1 1pt
\relax
\parshape=1 x 1pt
\parshape=x
\parshape=2 1pt 1pt
\par
\parshape=0
\begingroup\parshape=1 1pt 1pt \showthe\parshape\endgroup\showthe\parshape
\begingroup\global\parshape=1 1pt 1pt \endgroup\showthe\parshape\parshape=0
\setbox1=\vbox{\parshape=100000 }
\parshape=0
\setbox1=\hbox{\parshape=1 0pt 10pt }\showthe\parshape\parshape=0
\advance\parshape by 1
\looseness=1 \parshape=2 0pt 50pt 0pt 50pt \setbox1=\vbox{one two three four\par}\showbox1 \parshape=0 \looseness=0
\setbox1=\vbox{\parshape=2 0pt 80pt 5pt 70pt \noindent a$$b$$c\par}\showbox1
\setbox1=\vbox{\parshape=2 0pt 80pt 5pt 70pt \noindent a $$\showthe\displaywidth\showthe\displayindent b\eqno(1)$$ c\par}\showbox1
""")
P('hangindent hangafter', r"""
\T{hanging indentation}
\def\t{one two three four five six seven eight nine ten eleven twelve}
\setbox1=\vbox{\hsize=60pt \hangindent=10pt \hangafter=1 \noindent\t\par\showthe\hangindent\showthe\hangafter}\showbox1
\setbox1=\vbox{\hsize=60pt \hangindent=10pt \hangafter=-2 \noindent\t\par}\showbox1
\setbox1=\vbox{\hsize=60pt \hangindent=-10pt \hangafter=2 \noindent\t\par}\showbox1
\setbox1=\vbox{\hsize=60pt \hangindent=-10pt \hangafter=-1 \noindent\t\par}\showbox1
\setbox1=\vbox{\hsize=60pt \hangindent=100pt \hangafter=0 \noindent\t\par}\showbox1
\setbox1=\vbox{\hsize=60pt \hangindent=10pt \hangafter=100 \noindent\t\par}\showbox1
\setbox1=\vbox{\hsize=60pt \hangindent=10pt \hangafter=-100 \noindent\t\par}\showbox1
\setbox1=\vbox{\hsize=120pt \hangindent=10pt \hangafter=-1 \noindent a$$\showthe\displaywidth\showthe\displayindent b$$\hangindent=10pt \hangafter=1 c$$d$$\par}\showbox1
""")

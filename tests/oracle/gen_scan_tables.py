#!/usr/bin/env python3
"""The oracle's answers for the unit tests of scan.rs, cond.rs and buildtoks.rs.

Run with python3 -I.  It writes crates/engine/testdata/escape_oracle.tsv, see gen_escape, and
crates/engine/testdata/scan_oracle.tsv, one row per case:

    kind  mag  text  value  rest  errors

kind says how the text is read, and how the oracle is made to read it with the same routine:

    int     \\count1=TEXT|                     scan_int
    dimen   \\dimen1=TEXT|                     scan_dimen(false,false,false)
    glue    \\skip1=TEXT|                      scan_glue(glue_val)
    muglue  \\muskip1=TEXT|                    scan_glue(mu_val)
    edef    \\edef\\r{TEXT}|                    scan_toks(true,true), as TEXT is the body
    def     \\def\\r TEXT|                      scan_toks(true,false), and TEXT has the parameters too
    xdef    \\edef\\r TEXT|                     scan_toks(true,true), the same

value is the number, the glue as \\the shows it, or the list as \\detokenize or \\meaning shows it.  rest is
what is left before the |, and errors the lines starting with `! ` in order.  In a field, %XX stands
for a byte; a newline between errors is one such byte.

The machine in the tests has the state PREAMBLE gives, by hand: see the testkit in scan.rs.
"""
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "scan_oracle.tsv")

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\_=8
\catcode`\&=4 \catcode`\$=3 \catcode`\~=13
\scrollmode \tracingonline=0 \newlinechar=-1
\def\grab#1|{\gdef\rest{#1}}
\def\m{12}\def\mm{1}\def\n{pt}\def\e{}\def\s{ }\def\space{ }\def\pl{plus}
\def\a{x}\def\b{x}\def\cc{y}\let\z=\a \long\def\la{x}\def\aa#1{#1}\def\ab#1{#1}\def\ac#2{}
\protected\def\pp{x}\def\q{\pp}
\let\bg={\let\eg=}\let\xx=x
\chardef\chA=65 \mathchardef\mcB=1234
\tolerance=321 \hbadness=-5 \hsize=100pt \vsize=-3pt \baselineskip=12pt plus 1pt minus 2pt
\parskip=0pt plus 1fil minus 2fill \thinmuskip=3mu plus 1mu minus 2fill
\catcode`\!=12
\mag=MAGVALUE
\toksdef\tt=0 \everypar{ab}\everyhbox{}
"""

INT = [
    "0", "1", "123", "007", "-1", "+1", "--1", "+-+-1", "-+-1", " 7", "   7", " - 7", "- - 7", "1 ", "1  x", "1x", "1 2", "12a", "1\\relax", "1\\relax x", "1\\e", "1\\e 2", "1\\s", "1\\s\\s",
    "\\m", "\\m3", "\\m 3", "\\m\\m", "-\\m", "1\\m", "1\\m\\relax", "\\e 7", "\\e\\e7", "\\s 7", "\\s\\s7", "\\relax 7", "\\relax",
    "2147483647", "2147483648", "2147483649", "-2147483647", "-2147483648", "21474836470", "99999999999", "999999999999999", "12345678901", "2147483640", "214748364", "214748365", "1000000000", "9999999999",
    "'0", "'7", "'8", "'17", "'177", "'17777777777", "'20000000000", "'37777777777", "'777777777777", "-'17", "' 7", "'", "'x", "'\\m", "'7 x", "'7\\relax", "'07",
    "\"0", "\"F", "\"f", "\"G", "\"FF", "\"1A", "\"A1", "\"7FFFFFFF", "\"80000000", "\"FFFFFFFF", "\"FFFFFFFFFF", "-\"F", "\" 7", "\"", "\"x", "\"F x", "\"F\\relax", "\"\\m", "\"AB\\m", "\"ABCDEF", "\"abcdef", "\"AbC",
    "`a", "`A", "`1", "`\\a", "`\\A", "`\\1", "`\\relax", "`\\m", "`\\ab", "` ", "`  ", "`a ", "`a  x", "`ax", "`a\\relax", "`\\a ", "`\\a  x", "-`a", "`~", "`\\~", "`^^M", "`\\^^M", "`\\ ", "`^^@", "`^^ff", "`\\^^ff", "`{}", "`}{", "`\\{", "`\\}", "`#", "`$", "`&", "`_", "`^", "`%",
    "x", "", " ", "\\s", "a1", "-", "+", "- ", "-x", ".", ",", ".5", "1.5", "1,5", "\\relax 1",
    "{", "}", "$", "&", "#", "^", "_", "~",
    "\\tolerance", "-\\tolerance", "--\\tolerance", "\\tolerance 1", "\\tolerance x", "\\hbadness", "-\\hbadness", "\\hsize", "-\\hsize", "\\vsize", "\\baselineskip", "-\\baselineskip", "\\parskip", "\\thinmuskip", "-\\thinmuskip",
    "\\chA", "-\\chA", "\\chA 1", "\\mcB", "-\\mcB", "\\catcode`\\a", "\\catcode`\\{", "\\catcode`a", "\\catcode 300", "\\catcode -1", "\\catcode 256", "\\catcode 255", "\\catcode 0", "\\catcode", "\\catcode x", "\\catcode`\\a x",
    "\\prevdepth", "-\\prevdepth", "\\spacefactor", "\\prevgraf", "\\deadcycles", "\\insertpenalties", "\\interactionmode", "\\pagegoal", "\\pagetotal", "\\pagestretch", "\\pagefilstretch", "\\pagefillstretch", "\\pagefilllstretch", "\\pageshrink", "\\pagedepth",
    "\\parshape", "\\interlinepenalties", "\\clubpenalties", "\\widowpenalties", "\\displaywidowpenalties", "\\interlinepenalties 1", "\\interlinepenalties -1",
    "\\inputlineno x", "\\badness", "\\lastpenalty", "\\lastkern", "\\lastskip", "\\lastnodetype", "\\pdftexversion", "\\eTeXversion", "\\currentgrouplevel", "\\currentgrouptype", "\\currentiflevel", "\\currentiftype", "\\currentifbranch", "\\pdflastobj", "\\pdflastxform", "\\pdflastximage", "\\pdflastximagepages", "\\pdflastannot", "\\pdflastlink", "\\pdflastxpos", "\\pdflastypos", "\\pdfretval", "\\pdflastximagecolordepth", "\\pdfshellescape",
    "\\pdfprimitive\\tolerance", "-\\pdfprimitive\\tolerance", "\\pdfprimitive\\relax", "\\pdfprimitive\\a", "\\pdfprimitive x", "\\pdfprimitive\\undefinedname", "\\pdfprimitive\\chardef", "\\pdfprimitive\\pdfprimitive\\tolerance", "\\pdfprimitive 7", "\\pdfprimitive\\inputlineno", "\\pdfprimitive\\m",
    "\\ignorespaces 7", "\\everypar", "\\toks", "\\tt", "\\nullfont", "\\font", "\\textfont", "\\undefinedname",
    "\\hrule", "\\par", "\\number 7", "\\the\\tolerance", "\\number\\tolerance", "1\\the\\tolerance", "\\romannumeral 3", "\\string a", "\\string\\a", "1\\string 2",
]

DIMEN = [
    "1pt", "0pt", "-1pt", "+1pt", "--1pt", "1 pt", "1  pt", " 1pt", "1pt ", "1pt  x", "1ptx", "1pt\\relax", "1pt\\s", "1pt\\e", "1pt\\e x", "1\\n", "\\m pt", "\\m\\n", "\\mm\\m pt",
    "1in", "1pc", "1cm", "1mm", "1bp", "1dd", "1cc", "1nd", "1nc", "1sp", "1em", "1ex", "1px", "1PT", "1Pt", "1pT", "1IN", "1Pc", "1CM", "1Mm", "1bP", "1DD", "1cC", "1ND", "1nC", "1SP", "1EM", "1eX", "1PX",
    "2in", "3pc", "7cm", "11mm", "13bp", "17dd", "19cc", "23nd", "29nc", "31sp", "37em", "41ex", "43px",
    "0.5pt", ".5pt", "5.pt", "1.5pt", "1,5pt", ",5pt", "5,pt", ".pt", ",pt", ". 5pt", "1 .5pt", "1. 5pt", ".5 pt", "0.1pt", "0.01pt", "0.001pt", "0.0001pt", "0.00001pt", "0.000001pt", "0.0000076pt", "0.0000077pt", "0.00000762939453125pt", "0.00000762939453124pt", "0.999999pt", "0.99999pt", "0.1234567890123456789pt", "1.00000000000000000000001pt", "1.99999999999999999999pt", "0.33333333333333333333333pt",
    "0.5in", "0.5pc", "0.5cm", "0.5mm", "0.5bp", "0.5dd", "0.5cc", "0.5nd", "0.5nc", "0.5sp", "1.5sp", "0.9sp", "65536sp", "65535.9sp", "2147483647sp", "1073741823sp", "1073741824sp", "2147483648sp", "-1073741823sp", "-1073741824sp",
    "16383.99998pt", "16383.99999pt", "16384pt", "16383.999999pt", "-16383.99999pt", "-16384pt", "100000pt", "2147483647pt", "2147483648pt", "226.7in", "226.71in", "226.72in", "1365.33pc", "1365.34pc", "575.83cm", "575.84cm", "5758.3mm", "5758.4mm", "16322bp", "16323bp", "15312dd", "15313dd", "1276cc", "1277cc", "15355nd", "15356nd", "1279nc", "1280nc",
    "'7pt", "'17pt", "\"Apt", "\"1Fpt", "'7.5pt", "\"A.5pt", "'7,5pt", "`apt", "`a pt", "`\\a pt", "`a.5pt", "'77777777777pt", "\"FFFFFFFFpt", "99999999999pt", "99999999999.5pt", "99999999999x",
    "1", "1x", "", "x", "1 x", "1.5", "1.5x", ".", ",", "1p", "1p x", "1 p", "1px", "1pa", "1pa x", "1i", "1ix", "1c", "1m", "1b", "1d", "1n", "1s", "1e", "1p\\relax", "1\\relax", "1 \\relax", "1\\relax pt", "1pxx", "1inn", "1inch", "1ptpt", "1p t", "1 p t",
    "1fil", "1fill", "1true", "1true x", "1truex", "1 true pt", "1true pt", "1truept", "1 truept", "1TRUEPT", "1TrUe pT", "1true  pt", "1tru", "1tr", "1t", "1tx", "1truein", "1truepc", "1truecm", "1truemm", "1truebp", "1truedd", "1truecc", "1truend", "1truenc", "1truesp", "1trueem", "1trueex", "1truepx", "1.5truept", "1.5truein", "0.5truesp", "16383truept", "16384truept", "16383.99999truept", "2truex", "1true true pt", "-1truept", "--1truept", "-1.5truein",
    "\\hsize", "-\\hsize", "--\\hsize", "\\hsize x", "\\hsize 1", "\\vsize", "-\\vsize", "2\\hsize", "-2\\hsize", "2 \\hsize", "2.5\\hsize", "-2.5\\hsize", ".5\\hsize", "0.5\\hsize", "1\\hsize", "0\\hsize", "1.00001\\hsize", "163\\hsize", "164\\hsize", "163.84\\hsize", "163.83\\hsize", "2\\vsize", "-2\\vsize", "2\\hsize x", "2\\hsize\\relax", "2\\hsize ", "2\\hsize  x",
    "\\baselineskip", "-\\baselineskip", "2\\baselineskip", "2\\parskip", "\\parskip", "\\thinmuskip", "2\\thinmuskip", "-\\thinmuskip", "\\tolerance", "\\tolerance pt", "-\\tolerance pt", "\\tolerance\\hsize", "\\tolerance\\baselineskip", "\\tolerance x", "\\tolerance.5pt", "\\hbadness pt", "\\hbadness\\hsize", "\\chA pt", "\\chA\\hsize", "\\catcode`\\a pt", "\\catcode`\\a\\hsize", "`a\\hsize", "'7\\hsize", "\"A\\hsize", "\\m\\hsize", "\\m pt", "\\tolerance\\tolerance", "\\tolerance\\thinmuskip",
    "\\prevdepth", "\\pagegoal", "\\pagetotal", "\\pagedepth", "\\spacefactor", "\\spacefactor pt", "\\prevgraf pt", "\\deadcycles pt", "\\insertpenalties pt", "\\parshape pt", "\\interlinepenalties 1pt", "\\pdfprimitive\\hsize", "-\\pdfprimitive\\hsize", "2\\pdfprimitive\\hsize", "\\pdfprimitive\\relax", "\\pdfprimitive\\tolerance pt",
    "\\everypar", "\\nullfont", "\\font", "\\hrule", "\\lastkern", "\\lastpenalty pt", "\\lastskip", "\\lastnodetype pt", "\\inputlineno pt", "\\badness pt",
]

GLUE = [
    "1pt", "0pt", "-1pt", "1 pt", "1pt plus 1pt", "1pt minus 1pt", "1pt plus 1pt minus 1pt", "1pt minus 1pt plus 1pt", "1pt plus 1pt plus 1pt", "1pt minus 1pt minus 1pt", "1pt plus -1pt minus -1pt", "-1pt plus 1pt minus 1pt", "--1pt plus 1pt",
    "1pt plus1pt", "1ptplus1pt", "1pt  plus  1pt", "1pt PLUS 1pt", "1pt pLuS 1pt", "1pt MINUS 1pt", "1pt mInUs 1pt", "1pt p", "1pt pl", "1pt plu", "1pt pl x", "1pt plx", "1pt m", "1pt mi", "1pt min", "1pt minu", "1pt mix", "1pt p l u s 1pt", "1pt plus", "1pt minus", "1pt plus x", "1pt minus x", "1pt plus 1", "1pt minus 1", "1pt plus 1 x",
    "1pt \\pl 1pt", "1pt\\pl 1pt", "1pt\\s\\pl\\s 1pt", "1pt p\\e lus 1pt", "1pt pl\\e us 1pt", "1pt pl\\relax us", "1pt\\relax plus 1pt",
    "1pt plus 1fil", "1pt plus 1fill", "1pt plus 1filll", "1pt plus 1fillll", "1pt plus 1filllll", "1pt plus 1 fil", "1pt plus 1fil l", "1pt plus 1fil l l", "1pt plus 1fi", "1pt plus 1f", "1pt plus 1fix", "1pt plus 1FIL", "1pt plus 1fIlL", "1pt plus 1filx", "1pt plus 1fil x", "1pt plus 1fill x", "1pt plus 1fil  ", "1pt plus 1.5fil", "1pt plus .5fill", "1pt plus 16383.99999filll", "1pt plus 16384fil", "1pt plus -1fil", "1pt plus 1fil minus 1fill", "1pt plus 1fill minus 1fil", "1pt plus 1fil plus 1fil", "1pt minus 1filll", "1pt minus 1fillll", "1pt minus 16384fil", "1fil", "1pt fil",
    "1pt plus 1true pt", "1pt plus 1truefil", "1pt plus 1em", "1pt plus 1in minus 1cm", "1in plus 1cm minus 1mm", "1pt plus \\hsize", "1pt plus -\\hsize", "1pt plus 2\\hsize", "1pt minus \\hsize", "1pt plus \\baselineskip", "1pt plus \\thinmuskip", "1pt plus \\tolerance pt", "1pt plus \\tolerance fil", "1pt plus \\tolerance\\hsize", "1pt plus \\parskip",
    "\\baselineskip", "-\\baselineskip", "--\\baselineskip", "\\baselineskip x", "\\baselineskip plus 1pt", "\\baselineskip minus 1pt", "2\\baselineskip", "-2\\baselineskip", "2\\baselineskip plus 1pt", "2.5\\baselineskip", "\\parskip", "-\\parskip", "\\parskip plus 1pt", "2\\parskip plus 1fil",
    "\\hsize", "-\\hsize", "\\hsize plus 1pt", "-\\hsize plus 1pt", "2\\hsize", "-2\\hsize plus 1pt", "\\vsize minus 1pt", "\\tolerance pt", "-\\tolerance pt", "\\tolerance pt plus 1pt", "-\\tolerance pt plus 1pt", "\\tolerance\\hsize", "-\\tolerance\\hsize", "\\tolerance\\baselineskip", "\\tolerance", "\\tolerance x", "\\chA pt", "\\catcode`\\a pt plus 1pt",
    "\\thinmuskip", "-\\thinmuskip", "2\\thinmuskip", "\\thinmuskip plus 1pt", "\\tolerance\\thinmuskip", "1mu", "1pt plus 1mu", "1", "x", "", ".", "\\relax", "\\relax 1pt", "\\e 1pt", "\\s 1pt", "\\everypar", "\\nullfont", "\\hrule", "\\lastskip", "-\\lastskip", "\\lastskip plus 1pt", "\\lastkern", "-\\lastkern plus 1pt", "\\lastpenalty pt", "\\pdfprimitive\\baselineskip", "-\\pdfprimitive\\baselineskip", "\\pdfprimitive\\hsize plus 1pt", "\\pdfprimitive\\tolerance pt", "\\pagegoal plus 1pt", "\\prevdepth plus 1pt", "\\spacefactor pt plus 1pt",
]

MUGLUE = [
    "1mu", "0mu", "-1mu", "1 mu", "1MU", "1mU", "1.5mu", ".5mu", "1,5mu", "1mu plus 1mu", "1mu minus 1mu", "1mu plus 1mu minus 1mu", "1mu plus 1fil", "1mu plus 1fill minus 1filll", "1mu plus 1fillll", "1mu plus 1pt", "1mu minus 1pt", "1mu plus 1", "1mu minus 1", "1mu plus -1mu",
    "1pt", "1", "1x", "1m", "1m x", "1mx", "", "x", ".", "1em", "1ex", "1in", "1sp", "1true mu", "1fil", "1 x",
    "16383.99999mu", "16384mu", "2147483647mu", "1mu plus 16384mu", "'7mu", "\"Amu", "`amu",
    "\\thinmuskip", "-\\thinmuskip", "--\\thinmuskip", "\\thinmuskip plus 1mu", "2\\thinmuskip", "-2\\thinmuskip", "2.5\\thinmuskip", "2\\thinmuskip plus 1mu", ".5\\thinmuskip", "\\thinmuskip x",
    "\\baselineskip", "-\\baselineskip", "\\parskip", "2\\baselineskip", "-2\\baselineskip", "2\\baselineskip plus 1mu", "\\hsize", "-\\hsize", "2\\hsize", "2\\hsize plus 1mu", "\\hsize plus 1mu", "-\\hsize minus 1mu", "\\tolerance mu", "-\\tolerance mu", "\\tolerance mu plus 1mu", "\\tolerance\\thinmuskip", "-\\tolerance\\thinmuskip", "\\tolerance\\baselineskip", "\\tolerance\\hsize", "\\tolerance", "\\chA mu",
    "\\everypar", "\\nullfont", "\\relax", "\\hrule", "\\lastskip", "\\lastkern", "\\lastpenalty mu", "\\pdfprimitive\\thinmuskip", "\\pdfprimitive\\baselineskip",
]

EDEF = [
    "", "a", "a b", "{a}", "{{a}b}", "\\relax", "\\a", "\\a\\a", "\\aa{b}", "\\aa b", "\\z", "\\xx", "\\bg", "\\eg", "\\pp", "\\q", "\\noexpand\\a", "\\noexpand\\pp", "\\noexpand x", "\\expandafter\\noexpand\\a", "\\expandafter\\a\\a", "\\string\\a", "\\string a", "\\string\\ ", "\\string~", "\\string\\~", "\\string{", "\\string}", "\\string\\{", "\\string^^M", "\\string\\^^M",
    "\\number 12", "\\number-12", "\\number\\tolerance", "\\number`a", "\\number\"F", "\\number'7", "\\number\\hsize", "\\number x", "\\number", "\\romannumeral 14", "\\romannumeral 1994", "\\romannumeral 0", "\\romannumeral -1", "\\romannumeral 4999", "\\romannumeral 5000", "\\romannumeral 49", "\\romannumeral\\tolerance",
    "\\meaning\\a", "\\meaning x", "\\meaning\\relax", "\\meaning\\undefinedname", "\\meaning\\aa", "\\meaning\\la", "\\meaning\\pp", "\\meaning\\z", "\\meaning\\xx", "\\meaning\\bg", "\\meaning\\chA", "\\meaning\\mcB", "\\meaning\\tolerance", "\\meaning\\hsize", "\\meaning\\baselineskip", "\\meaning~", "\\meaning{", "\\meaning}", "\\meaning#", "\\meaning^", "\\meaning $", "\\meaning&", "\\meaning_", "\\meaning\\ ", "\\meaning\\ifnum", "\\meaning\\fi", "\\meaning\\else", "\\meaning\\or", "\\meaning\\meaning",
    "\\the\\tolerance", "\\the\\hbadness", "\\the\\hsize", "\\the\\vsize", "\\the\\baselineskip", "\\the\\parskip", "\\the\\thinmuskip", "\\the\\everypar", "\\the\\everyhbox", "\\the\\chA", "\\the\\mcB", "\\the\\catcode`\\a", "\\the\\inputlineno", "\\the\\prevdepth", "\\the\\spacefactor", "\\the\\pagegoal", "\\the\\interactionmode", "\\the\\pdftexversion", "\\the\\eTeXversion", "\\the\\relax", "\\the x", "\\the\\a", "\\the\\undefinedname", "\\the", "\\the\\number 1", "\\the\\the\\tolerance", "\\the\\hrule", "\\the{", "\\the\\lastskip", "\\the\\lastkern", "\\the\\lastpenalty", "\\the\\lastnodetype", "\\the\\currentiflevel", "\\the\\ignorespaces", "\\the\\pdfprimitive\\tolerance", "\\the\\pdfprimitive\\relax", "\\the\\pdfprimitive\\a", "\\the\\pdfprimitive\\pdfprimitive\\everypar", "\\the\\parshape", "\\the\\interlinepenalties 3",
    "\\eTeXrevision", "\\pdftexrevision",
    "\\iftrue T\\else F\\fi", "\\iffalse T\\else F\\fi", "\\iftrue T\\fi", "\\iffalse T\\fi", "\\iftrue T\\else F\\else G\\fi", "\\iffalse T\\else F\\else G\\fi", "\\iftrue\\iftrue A\\fi B\\else C\\fi", "\\iffalse\\iftrue A\\else B\\fi C\\else D\\fi", "\\iffalse\\iffalse A\\else B\\fi C\\else D\\fi", "\\iftrue\\iffalse A\\else B\\fi C\\else D\\fi",
    "\\fi", "\\else", "\\or", "\\iftrue T\\or U\\fi", "\\iffalse T\\or U\\else V\\fi", "\\iffalse T\\or U\\or W\\else V\\fi", "\\iftrue\\fi\\fi", "\\iffalse\\else\\else\\fi", "\\iftrue\\else\\fi\\else", "\\iftrue\\else\\or\\fi", "\\iffalse\\or\\fi",
    "\\ifnum 1<2 T\\else F\\fi", "\\ifnum 2<1 T\\else F\\fi", "\\ifnum 1=1 T\\else F\\fi", "\\ifnum 1=2 T\\else F\\fi", "\\ifnum 2>1 T\\else F\\fi", "\\ifnum 1>2 T\\else F\\fi", "\\ifnum 1 < 2 T\\fi", "\\ifnum1<2T\\fi", "\\ifnum 1 2 T\\else F\\fi", "\\ifnum 1 x2 T\\else F\\fi", "\\ifnum 1!2 T\\else F\\fi", "\\ifnum 1;2 T\\else F\\fi", "\\ifnum 1?2 T\\else F\\fi", "\\ifnum x<2 T\\else F\\fi", "\\ifnum 1<x T\\else F\\fi", "\\ifnum\\tolerance=321 T\\else F\\fi", "\\ifnum\\tolerance>\\hbadness T\\else F\\fi", "\\ifnum\\m<\\m3 T\\else F\\fi", "\\ifnum 1<\\m\\space T\\else F\\fi", "\\ifnum-1<0 T\\fi", "\\ifnum 2147483647>2147483646 T\\fi", "\\ifnum 2147483648=2147483647 T\\fi", "\\ifnum\\hsize=6553600 T\\fi", "\\ifnum 1<2\\relax T\\fi", "\\ifnum 1<2\\e T\\fi", "\\ifnum 1=1\\ifnum 2=2 T\\else F\\fi\\else G\\fi", "\\ifnum 1=2\\ifnum 2=2 T\\else F\\fi\\else G\\fi", "\\ifnum\\ifnum 1=1 1\\else 2\\fi=1 T\\else F\\fi",
    "\\ifdim 1pt<2pt T\\else F\\fi", "\\ifdim 2pt<1pt T\\else F\\fi", "\\ifdim 1pt=1pt T\\else F\\fi", "\\ifdim 1pt>2pt T\\else F\\fi", "\\ifdim 1pt 2pt T\\else F\\fi", "\\ifdim 1pt!2pt T\\else F\\fi", "\\ifdim\\hsize=100pt T\\else F\\fi", "\\ifdim\\hsize>\\vsize T\\else F\\fi", "\\ifdim 1in=72.27pt T\\else F\\fi", "\\ifdim 1pt<x T\\else F\\fi", "\\ifdim\\baselineskip=12pt T\\else F\\fi", "\\ifdim\\tolerance pt=321pt T\\else F\\fi", "\\ifdim 16384pt=16383.99998pt T\\else F\\fi",
    "\\ifodd 1 T\\else F\\fi", "\\ifodd 2 T\\else F\\fi", "\\ifodd -1 T\\else F\\fi", "\\ifodd 0 T\\else F\\fi", "\\ifodd\\tolerance T\\else F\\fi", "\\ifodd x T\\else F\\fi",
    "\\ifvmode T\\else F\\fi", "\\ifhmode T\\else F\\fi", "\\ifmmode T\\else F\\fi", "\\ifinner T\\else F\\fi",
    "\\if aa T\\else F\\fi", "\\if ab T\\else F\\fi", "\\if a\\a T\\else F\\fi", "\\if x\\a T\\else F\\fi", "\\if\\a x T\\else F\\fi", "\\if\\relax\\relax T\\else F\\fi", "\\if\\relax a T\\else F\\fi", "\\if a\\relax T\\else F\\fi", "\\if\\relax\\undefinedname T\\else F\\fi", "\\if\\xx x T\\else F\\fi", "\\if~~ T\\else F\\fi", "\\if~a T\\else F\\fi", "\\if\\noexpand~\\noexpand~ T\\else F\\fi", "\\if\\noexpand\\a\\noexpand\\b T\\else F\\fi", "\\if\\noexpand\\a x T\\else F\\fi", "\\if\\noexpand~x T\\else F\\fi", "\\if{{ T\\else F\\fi}}", "\\if\\bg\\bg T\\else F\\fi", "\\if\\bg{ T\\else F\\fi}", "\\if\\chA A T\\else F\\fi", "\\if\\chA\\chA T\\else F\\fi", "\\if\\space\\space T\\else F\\fi", "\\if\\iftrue a\\fi a T\\else F\\fi", "\\if\\pp x T\\else F\\fi", "\\if\\ifnum 1<2 a\\fi b T\\else F\\fi",
    "\\ifcat aa T\\else F\\fi", "\\ifcat ab T\\else F\\fi", "\\ifcat a1 T\\else F\\fi", "\\ifcat 12 T\\else F\\fi", "\\ifcat a\\a T\\else F\\fi", "\\ifcat\\relax\\relax T\\else F\\fi", "\\ifcat\\relax a T\\else F\\fi", "\\ifcat~~ T\\else F\\fi", "\\ifcat\\noexpand~\\noexpand~ T\\else F\\fi", "\\ifcat\\noexpand~a T\\else F\\fi", "\\ifcat{{ T\\else F\\fi}}", "\\ifcat{} T\\else F\\fi", "\\ifcat\\bg{ T\\else F\\fi}", "\\ifcat\\xx x T\\else F\\fi", "\\ifcat\\chA\\chA T\\else F\\fi", "\\ifcat\\chA A T\\else F\\fi", "\\ifcat\\noexpand\\a\\relax T\\else F\\fi", "\\ifcat\\noexpand\\a\\noexpand\\b T\\else F\\fi", "\\ifcat\\space\\space T\\else F\\fi", "\\ifcat\\undefinedname\\undefinedname T\\else F\\fi",
    "\\ifx aa T\\else F\\fi", "\\ifx ab T\\else F\\fi", "\\ifx a1 T\\else F\\fi", "\\ifx\\a\\a T\\else F\\fi", "\\ifx\\a\\b T\\else F\\fi", "\\ifx\\a\\cc T\\else F\\fi", "\\ifx\\a\\z T\\else F\\fi", "\\ifx\\a\\la T\\else F\\fi", "\\ifx\\a\\pp T\\else F\\fi", "\\ifx\\aa\\ab T\\else F\\fi", "\\ifx\\aa\\ac T\\else F\\fi", "\\ifx\\e\\e T\\else F\\fi", "\\ifx\\e\\a T\\else F\\fi", "\\ifx\\a\\e T\\else F\\fi", "\\ifx\\relax\\relax T\\else F\\fi", "\\ifx\\relax a T\\else F\\fi", "\\ifx\\undefinedname\\undefinedname T\\else F\\fi", "\\ifx\\undefinedname\\otherundefinedname T\\else F\\fi", "\\ifx\\undefinedname\\relax T\\else F\\fi", "\\ifx\\xx x T\\else F\\fi", "\\ifx\\bg{ T\\else F\\fi}", "\\ifx{\\bg T\\else F\\fi}", "\\ifx\\bg\\eg T\\else F\\fi", "\\ifx~~ T\\else F\\fi", "\\ifx~\\undefinedname T\\else F\\fi", "\\ifx\\chA\\chA T\\else F\\fi", "\\ifx\\tolerance\\tolerance T\\else F\\fi", "\\ifx\\tolerance\\hbadness T\\else F\\fi", "\\ifx\\ifnum\\ifnum T\\else F\\fi", "\\ifx\\fi\\fi T\\else F\\fi", "\\ifx\\ifnum\\fi T\\else F\\fi", "\\ifx\\else\\or T\\else F\\fi", "\\ifx\\noexpand\\noexpand T\\else F\\fi", "\\ifx\\a\\m T\\else F\\fi", "\\ifx\\s\\s T\\else F\\fi",
    "\\unless\\iftrue T\\else F\\fi", "\\unless\\iffalse T\\else F\\fi", "\\unless\\ifnum 1<2 T\\else F\\fi", "\\unless\\ifnum 2<1 T\\else F\\fi", "\\unless\\ifx\\a\\b T\\else F\\fi", "\\unless\\if ab T\\else F\\fi", "\\unless\\ifcase 1 T\\fi", "\\unless\\fi", "\\unless\\else", "\\unless\\relax", "\\unless x", "\\unless\\a", "\\unless\\unless\\iftrue T\\fi", "\\unless\\ifcsname a\\endcsname T\\else F\\fi", "\\unless\\ifdefined\\a T\\else F\\fi", "\\unless\\ifvmode T\\else F\\fi", "\\expandafter\\unless\\iftrue T\\else F\\fi", "\\unless\\expandafter\\iftrue T\\fi",
    "\\ifdefined\\a T\\else F\\fi", "\\ifdefined\\undefinedname T\\else F\\fi", "\\ifdefined\\relax T\\else F\\fi", "\\ifdefined x T\\else F\\fi", "\\ifdefined~ T\\else F\\fi", "\\ifdefined\\fi T\\else F\\fi", "\\ifdefined\\ifnum T\\else F\\fi",
    "\\ifcsname a\\endcsname T\\else F\\fi", "\\ifcsname undefinedname\\endcsname T\\else F\\fi", "\\ifcsname relax\\endcsname T\\else F\\fi", "\\ifcsname\\endcsname T\\else F\\fi", "\\ifcsname \\endcsname T\\else F\\fi", "\\ifcsname\\a\\endcsname T\\else F\\fi", "\\ifcsname\\e\\endcsname T\\else F\\fi", "\\ifcsname notdefined\\endcsname T\\else F\\fi\\ifcsname notdefined\\endcsname T\\else F\\fi", "\\ifcsname a\\relax b\\endcsname T\\else F\\fi", "\\ifcsname\\relax\\endcsname T\\else F\\fi", "\\ifcsname a\\fi", "\\ifcsname x\\endcsname\\ifincsname A\\else B\\fi\\fi", "\\ifcsname\\ifincsname a\\else b\\fi\\endcsname T\\else F\\fi", "\\ifincsname T\\else F\\fi", "\\csname\\ifincsname a\\else b\\fi\\endcsname", "\\csname\\ifcsname a\\endcsname\\ifincsname a\\else b\\fi\\fi\\endcsname", "\\ifcsname\\csname a\\endcsname\\endcsname T\\else F\\fi",
    "\\ifpdfabsnum -1=1 T\\else F\\fi", "\\ifpdfabsnum 1=-1 T\\else F\\fi", "\\ifpdfabsnum -1<2 T\\else F\\fi", "\\ifpdfabsnum 2>-1 T\\else F\\fi", "\\ifpdfabsnum -2<1 T\\else F\\fi", "\\ifpdfabsnum 1 2 T\\else F\\fi", "\\ifpdfabsnum 1!2 T\\else F\\fi", "\\ifpdfabsnum x=0 T\\else F\\fi", "\\ifpdfabsdim -1pt=1pt T\\else F\\fi", "\\ifpdfabsdim 1pt=-1pt T\\else F\\fi", "\\ifpdfabsdim -1pt<2pt T\\else F\\fi", "\\ifpdfabsdim 2pt>-1pt T\\else F\\fi", "\\ifpdfabsdim 1pt 2pt T\\else F\\fi", "\\ifpdfabsdim 1pt!2pt T\\else F\\fi", "\\ifpdfabsdim\\vsize=3pt T\\else F\\fi", "\\unless\\ifpdfabsnum -1=1 T\\else F\\fi",
    "\\ifpdfprimitive\\relax T\\else F\\fi", "\\ifpdfprimitive\\a T\\else F\\fi", "\\ifpdfprimitive\\undefinedname T\\else F\\fi", "\\ifpdfprimitive\\z T\\else F\\fi", "\\ifpdfprimitive\\xx T\\else F\\fi", "\\ifpdfprimitive x T\\else F\\fi", "\\ifpdfprimitive~ T\\else F\\fi", "\\ifpdfprimitive\\ifnum T\\else F\\fi", "\\ifpdfprimitive\\fi T\\else F\\fi", "\\ifpdfprimitive\\tolerance T\\else F\\fi", "\\ifpdfprimitive\\ T\\else F\\fi", "\\ifpdfprimitive\\/ T\\else F\\fi", "\\ifpdfprimitive\\- T\\else F\\fi", "\\unless\\ifpdfprimitive\\relax T\\else F\\fi",
    "\\ifcase 0 A\\or B\\or C\\else D\\fi", "\\ifcase 1 A\\or B\\or C\\else D\\fi", "\\ifcase 2 A\\or B\\or C\\else D\\fi", "\\ifcase 3 A\\or B\\or C\\else D\\fi", "\\ifcase 4 A\\or B\\or C\\else D\\fi", "\\ifcase -1 A\\or B\\or C\\else D\\fi", "\\ifcase 2147483647 A\\or B\\fi", "\\ifcase 0 A\\or B\\fi", "\\ifcase 1 A\\or B\\fi", "\\ifcase 2 A\\or B\\fi", "\\ifcase 0 \\fi", "\\ifcase 1 \\fi", "\\ifcase 1 A\\else B\\fi", "\\ifcase 0 A\\else B\\or C\\fi", "\\ifcase 1 A\\else B\\or C\\fi", "\\ifcase 2 A\\else B\\or C\\fi", "\\ifcase x A\\or B\\fi", "\\ifcase\\tolerance A\\or B\\else C\\fi", "\\ifcase 1 \\iftrue A\\or B\\else C\\fi\\or D\\else E\\fi", "\\ifcase 1 \\ifcase 0 A\\or B\\fi\\or D\\else E\\fi", "\\ifcase 0 \\iftrue A\\or B\\else C\\fi\\or D\\else E\\fi", "\\ifcase 1 A\\or\\ifcase 1 B\\or C\\fi\\or D\\fi",
    "\\ifcase 0 A\\or B\\else C\\else D\\fi", "\\ifcase 1 A\\or B\\else C\\else D\\fi", "\\ifcase 2 A\\or B\\else C\\else D\\fi", "\\ifcase 2 A\\else C\\else D\\fi",
    "\\iffalse\\ifnum\\fi x", "\\iffalse\\ifx\\else y\\fi\\else x\\fi", "\\iffalse\\undefinedname\\else x\\fi", "\\iffalse\\ifcase\\or\\else\\fi\\else x\\fi", "\\iffalse\\a\\b\\cc\\else x\\fi", "\\iffalse\\ifx\\fi\\else x\\fi", "\\iffalse\\unless\\ifx\\fi\\else x\\fi", "\\iffalse\\ifcsname\\fi\\else x\\fi", "\\iffalse{\\else}\\fi", "\\iftrue{\\else}\\fi}", "\\iffalse#\\else x\\fi",
    "\\expandafter\\ifx\\a x T\\else F\\fi", "\\expandafter\\ifx\\a\\a T\\else F\\fi", "\\expandafter\\ifnum\\m=12 T\\else F\\fi", "\\ifnum\\expandafter\\m\\m=1212 T\\else F\\fi", "\\ifnum 1<2 \\expandafter T\\else\\expandafter F\\fi", "\\ifnum 2<1 \\expandafter T\\else\\expandafter F\\fi", "\\expandafter\\meaning\\iftrue\\fi", "\\expandafter\\meaning\\iffalse\\else\\fi", "\\expandafter\\meaning\\ifcase 1 \\or\\else\\fi",
    "\\ifnum\\currentiflevel=1 T\\else F\\fi", "\\iftrue\\the\\currentiflevel\\fi", "\\iftrue\\iftrue\\the\\currentiflevel\\fi\\fi", "\\iftrue\\the\\currentiftype\\fi", "\\iffalse\\else\\the\\currentiftype\\fi", "\\unless\\iftrue\\else\\the\\currentiftype\\fi", "\\ifcase 1\\or\\the\\currentiftype\\fi", "\\iftrue\\the\\currentifbranch\\else\\fi", "\\iffalse\\else\\the\\currentifbranch\\fi", "\\ifcase 0 \\the\\currentifbranch\\or\\fi", "\\ifcase 1 \\or\\the\\currentifbranch\\fi", "\\ifcase 1 \\else\\the\\currentifbranch\\fi", "\\the\\currentifbranch", "\\the\\currentiftype", "\\the\\currentiflevel", "\\ifnum\\currentifbranch=0 \\the\\currentifbranch\\fi",
    "\\if\\fi", "\\ifx\\fi", "\\ifnum\\fi", "\\ifdim\\fi", "\\ifodd\\fi", "\\ifcase\\fi", "\\ifnum\\else", "\\ifnum 1\\fi", "\\ifnum 1<\\fi", "\\ifdim 1pt\\fi", "\\ifcat\\fi", "\\if a\\fi", "\\ifcat a\\fi", "\\ifx a\\fi", "\\ifdefined\\fi", "\\ifpdfprimitive\\fi", "\\ifpdfabsnum\\fi", "\\ifpdfabsdim\\fi", "\\ifnum\\or",
    "#", "##", "a#b", "#1", "\\a#", "\\the\\everypar#",
]

DEF = [
    "{}", "{a}", "#1{a}", "#1{#1}", "#1#2{#2#1}", "#1#2#3#4#5#6#7#8#9{#9}", "#1#2#3#4#5#6#7#8#9#", "x{a}", "x#1{a}", "#1x{a}", "#1x#2y{#2#1}", "#{a}", "#1#{a}", "x#{a}", "x y#1 z{a}",
    "#1#2#3#4#5#6#7#8#9#1{a}", "#1#2#3#4#5#6#7#8#9#x{a}", "#1#2#3#4#5#6#7#8#9#{a}", "#2{a}", "#0{a}", "#x{a}", "#1#3{a}", "#1#1{a}", "##{a}", "#\\a{a}", "# {a}",
    "}", "x}", "#1}", "#}", "{#}", "{#x}", "{#0}", "{#1}", "#1{#2}", "#1{#0}", "#1{#x}", "#1{##}", "{##}", "{###}", "{####}", "#1{#1#1}", "#1{##1}", "#1{###1}", "#1{#}", "{{#}}", "#1{{#1}}", "#1{#\\a}", "{#\\a}", "{#{}", "#9{a}",
    "{a}b", "{{a}}", "{{a}{b}}", "{a{b}c}", "{}x", "\\a{a}", "\\relax{a}", "{\\a}", "{\\relax}", "{\\iftrue}", "~{a}", "{~}", "{$&^_}", "\\bg a}", "\\bg}", "{\\eg}", "{\\bg}}", "{\\aa{b}}",
    "{\\the\\tolerance}", "{\\number 1}", "{\\ifnum 1<2 T\\fi}", "{\\noexpand\\a}", "{\\pp}", "{\\s}", "{\\expandafter\\a\\a}", "#1\\a#2\\b{#1#2}",
]

XDEF = [
    "{}", "{a}", "#1{a}", "#1{#1}", "#1{##}", "{##}", "{#}", "{#x}", "{#1}", "#1{#2}", "#1{#\\a}", "#1{#\\m}", "#2{a}", "{\\a}", "{\\the\\tolerance}", "{\\the\\everypar}", "#1{\\the\\everypar#1}", "{\\number\\tolerance}", "{\\noexpand\\a}", "{\\noexpand#}", "{\\noexpand\\a#}", "{\\pp}", "{\\q}", "{\\a\\pp\\a}", "{\\expandafter\\a\\pp}", "{\\iftrue T\\else F\\fi}", "{\\aa{b}}", "{\\aa{\\aa{\\a}}}", "{\\string\\a}", "{\\meaning\\a}", "{\\the\\baselineskip}", "{\\the\\thinmuskip}", "{\\the x}", "{\\bg}", "\\bg}", "{{\\a}}", "{\\m\\m}", "#1#2{#2\\a#1}", "#1{#\\m}", "#1{#\\e 1}", "#1#2{#\\e 2}", "{\\undefinedname}", "{\\the\\tolerance\\the\\hsize}", "{\\csname a\\endcsname}", "{\\csname undefinedname\\endcsname}", "{\\noexpand\\noexpand}", "{\\noexpand~}", "{~}", "{\\the\\inputlineno}", "{\\romannumeral 9}",
]

KINDS = {
    "int": (INT, "\\count1=0 \\afterassignment\\grab\\count1=%s|\\immediate\\write16{V:%d:\\number\\count1:}"),
    "dimen": (DIMEN, "\\dimen1=0pt \\afterassignment\\grab\\dimen1=%s|\\immediate\\write16{V:%d:\\number\\dimen1:}"),
    "glue": (GLUE, "\\skip1=0pt \\afterassignment\\grab\\skip1=%s|\\immediate\\write16{V:%d:\\the\\skip1:}"),
    "muglue": (MUGLUE, "\\muskip1=0mu \\afterassignment\\grab\\muskip1=%s|\\immediate\\write16{V:%d:\\the\\muskip1:}"),
    "edef": (EDEF, "\\afterassignment\\grab\\edef\\r{%s}|\\immediate\\write16{V:%d:\\detokenize\\expandafter{\\r}:}"),
    "def": (DEF, "\\afterassignment\\grab\\def\\r %s|\\immediate\\write16{V:%d:\\meaning\\r:}"),
    "xdef": (XDEF, "\\afterassignment\\grab\\edef\\r %s|\\immediate\\write16{V:%d:\\meaning\\r:}"),
}

BATCH = 1  # a case that leaves a conditional or a group open must not disturb the next


def run_oracle(lines, mag, tmp):
    path = os.path.join(tmp, "t.tex")
    with open(path, "wb") as f:
        f.write((PREAMBLE.replace("MAGVALUE", str(mag)) + "\n".join(lines) + "\n\\csname bye\\endcsname\\end\n").encode("latin-1"))
    env = dict(os.environ, max_print_line="1000000", error_line="254", half_error_line="238")
    subprocess.run(["pdftex", "-ini", "-etex", "-interaction=scrollmode", "t.tex"], cwd=tmp, env=env,
                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    with open(os.path.join(tmp, "t.log"), "rb") as f:
        return f.read().decode("latin-1")


def field(s):
    out = []
    for c in s:
        o = ord(c)
        if c == "%" or o < 32 or o >= 127:
            out.append("%%%02X" % o)
        else:
            out.append(c)
    return "".join(out)


def gen(kind, mag, texts, tmpl, out, tmp):
    for start in range(0, len(texts), BATCH):
        batch = texts[start:start + BATCH]
        lines = []
        for k, text in enumerate(batch):
            lines.append("\\gdef\\rest{}\\immediate\\write16{CASE:%d:}" % k)
            lines.append(tmpl % (text, k) + "\\immediate\\write16{R:%d:\\detokenize\\expandafter{\\rest}:}\\immediate\\write16{E:%d:}" % (k, k))
        log = run_oracle(lines, mag, tmp)
        parts = re.split(r"^CASE:(\d+):$", log, flags=re.M)
        got = (len(parts) - 1) // 2
        for k, text in enumerate(batch):
            if k >= got:
                print("%s: no result for %r" % (kind, text))
                continue
            body = parts[2 * k + 2]
            m = re.search(r"^V:%d:(.*):$" % k, body, flags=re.M)
            r = re.search(r"^R:%d:(.*):$" % k, body, flags=re.M)
            if not m or not r or ("E:%d:" % k) not in body:
                print("%s: skipped %r" % (kind, text))
                continue
            errors = [l for l in body.split("\n") if l.startswith("! ")]
            if any("\\grab" in l or "\\rest" in l for l in errors):
                print("%s: skipped %r, which upsets the oracle's own harness" % (kind, text))
                continue
            value = m.group(1)
            if kind in ("def", "xdef"):
                value = value[len("macro:"):]
            out.append((kind, mag, text, value, r.group(1), "\n".join(errors)))


def gen_escape(tmp):
    """escape_oracle.tsv: \\pdfescapestring, \\pdfescapename, \\pdfescapehex of each byte, then
    \\pdfunescapehex of some texts, and \\pdfcreationdate under SOURCE_DATE_EPOCH."""
    doc = [r"\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \batchmode"]
    for k in range(256):
        h = "%02X" % k
        doc.append("\\immediate\\write16{B:%d:\\pdfescapehex{\\pdfescapestring{\\pdfunescapehex{%s}}}:"
                   "\\pdfescapehex{\\pdfescapename{\\pdfunescapehex{%s}}}:\\pdfescapehex{\\pdfescapehex{\\pdfunescapehex{%s}}}:}" % (k, h, h, h))
    unescape = ["", "0", "4", "41", "414", "4142", "4 1", "4x1", "xyz", "6a6A", "a", "F", "fF", "0g0", "00", "4G", "G4", "-41", "414243444"]
    for k, u in enumerate(unescape):
        doc.append("\\immediate\\write16{U:%d:\\pdfescapehex{\\pdfunescapehex{%s}}:}" % (k, u))
    doc.append("\\csname bye\\endcsname\\end")
    with open(os.path.join(tmp, "e.tex"), "w") as f:
        f.write("\n".join(doc) + "\n")
    env = dict(os.environ, max_print_line="1000000")
    subprocess.run(["pdftex", "-ini", "-etex", "-interaction=batchmode", "e.tex"], cwd=tmp, env=env,
                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    log = open(os.path.join(tmp, "e.log"), "rb").read().decode("latin-1")
    rows = []
    for k, s, n, h in re.findall(r"^B:(\d+):([0-9A-F]*):([0-9A-F]*):([0-9A-F]*):$", log, flags=re.M):
        de = lambda x: bytes.fromhex(x).decode("latin-1")
        rows.append("B\t%s\t%s\t%s\t%s" % (k, field(de(s)), field(de(n)), field(de(h))))
    if len(rows) != 256:
        sys.exit("escape: %d rows" % len(rows))
    for k, r in re.findall(r"^U:(\d+):([0-9A-F]*):$", log, flags=re.M):
        rows.append("U\t%s\t%s" % (field(unescape[int(k)]), field(bytes.fromhex(r).decode("latin-1"))))
    for epoch in (0, 1, 59, 86399, 951782400, 4102444800, 1234567890):
        with open(os.path.join(tmp, "d.tex"), "w") as f:
            f.write(r"\catcode`\{=1 \catcode`\}=2 \batchmode\immediate\write16{D:\pdfcreationdate:}\csname bye\endcsname\end" + "\n")
        env = dict(os.environ, SOURCE_DATE_EPOCH=str(epoch))
        subprocess.run(["pdftex", "-ini", "-etex", "-interaction=batchmode", "d.tex"], cwd=tmp, env=env,
                       stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        d = re.search(r"^D:(.*):$", open(os.path.join(tmp, "d.log"), "rb").read().decode("latin-1"), flags=re.M).group(1)
        rows.append("D\t%d\t%s" % (epoch, d))
    with open(os.path.join(os.path.dirname(OUT), "escape_oracle.tsv"), "wb") as f:
        f.write(("\n".join(rows) + "\n").encode("latin-1"))
    print("escape:", len(rows), "rows")


def main():
    rows = []
    with tempfile.TemporaryDirectory() as tmp:
        gen_escape(tmp)
        for kind, (texts, tmpl) in KINDS.items():
            gen(kind, 1000, texts, tmpl, rows, tmp)
        for mag in (2000, 500, 1, 32768, 1500):
            gen("dimen", mag, ["1truept", "1.5truept", "1truein", "1truecm", "0.1truesp", "16truept", "-3.3truebp", "1000truept", "16383truept", "1true em", "1truepx"], KINDS["dimen"][1], rows, tmp)
    with open(OUT, "wb") as f:
        for kind, mag, text, value, rest, errors in rows:
            f.write(("\t".join([kind, str(mag), field(text), field(value), field(rest), field(errors)]) + "\n").encode("latin-1"))
    print("scan:", len(rows), "cases")


if __name__ == "__main__":
    main()

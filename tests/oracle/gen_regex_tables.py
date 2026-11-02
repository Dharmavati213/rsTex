#!/usr/bin/env python3
"""The oracle's answers for \\pdfmatch and \\pdflastmatch, for the regex engine in buildtoks.rs.

Run with python3 -I.  It writes crates/engine/testdata/regex_oracle.tsv:

    pattern  text  icase  subcount  result  lastmatch0  lastmatch1 ...

result is 1, 0, or the message of a bad pattern.  A lastmatch is what \\pdflastmatch k gives, and those
after the last that is not -1-> are left out.  In a field, % followed by two hex digits stands for a
byte that would be awkward, including % itself.

The cases are the hand-picked list below, plus random ones made from a fixed seed.  They are all
synthetic.  A pattern that crashes the oracle or that it does not finish is reported and left out: both
happen, in glibc.  Arguments:

    --random N   how many random cases (default 1000)
    --seed S     the seed (default 20261102)
    --refs P     the chance that a random pattern has a back-reference added (default 0)
    --out PATH   where to write, instead of the table in testdata; for longer runs that are not kept
"""
import os
import random
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "regex_oracle.tsv")

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\batchmode \newlinechar=-1
\def\case#1#2#3#4#5{\immediate\write16{CASE:#1:}%
  \edef\r{\pdfmatch #4subcount #5{\pdfunescapehex{#2}}{\pdfunescapehex{#3}}}%
  \immediate\write16{RES:#1:\r:}%
  \count1=0
  \loop\ifnum\count1<12 \immediate\write16{LM:#1:\the\count1:\pdfescapehex{\pdflastmatch\count1}:}%
  \advance\count1 by 1 \repeat}
\def\loop#1\repeat{\def\body{#1}\iterate}
\def\iterate{\body\expandafter\iterate\fi}
\let\iterate\iterate
"""
# plain's \loop, with a working \else-less end
PREAMBLE = PREAMBLE.replace(r"""\def\iterate{\body\expandafter\iterate\fi}
\let\iterate\iterate
""", r"""\def\iterate{\body\let\next\iterate\else\let\next\relax\fi\next}
""")

# (pattern, text, icase, subcount); subcount None is 10
HAND = [
    # literals, alternatives, longest match
    ("a", "a"), ("a", "b"), ("a", "bab"), ("abc", "xabcy"), ("", ""), ("", "abc"), ("a|b", "b"), ("a|ab", "ab"),
    ("ab|a", "ab"), ("a|ab|abc", "abcd"), ("(a|ab)(c|bcd)(d*)", "abcd"), ("(a|ab)(c|bcd)", "abcd"), ("|a", "a"), ("a|", "a"),
    ("a||b", "b"), ("(|a)", "a"), ("(a|)", "a"), ("(|a)b", "ab"), ("(a|)b", "ab"), ("()", ""), ("()", "a"), ("()a", "a"), ("a()", "a"),
    ("(())", ""), ("((a))", "a"), ("(((a)))", "a"), ("((a)(b))", "ab"), ("((a))\\1", "aa"), ("((a))\\2", "aa"),
    # stars and friends
    ("a*", ""), ("a*", "aaa"), ("a*", "baaa"), ("a+", "baaa"), ("a?", "aa"), ("(a*)", "aaa"), ("(a*)*", "aaa"), ("(a*)+", "aaa"),
    ("(a+)*", "aaa"), ("(a?)*", "aaa"), ("(a?)+", "aaa"), ("((a?))*", "aa"), ("(a|b)*", "abab"), ("(a|b)*c", "ababc"), ("(a*)(a*)", "aaa"),
    ("(a*)(a+)", "aaa"), ("(a+)(a*)", "aaa"), ("(a*)b*", "aabb"), ("(a*|b)*", "aabb"), ("(a|ab)*", "abab"), ("(ab|a)*", "abab"),
    ("(a*)*b", "aab"), ("(a*b*)*", "abab"), ("(a|b|c)*(b|c)", "abcbc"), ("a**", "aaa"), ("a*+", "aaa"), ("a+*", "aaa"), ("a?*", "aaa"), ("a*?", "aaa"),
    ("(a*)*(a*)*", "aaa"), ("(a|())*", "aa"), ("(()|a)*", "aa"), ("(()|a)+b", "aab"), ("()*", ""), ("()*a", "a"), ("()+a", "a"), ("(a)*(b)*", "ab"),
    ("(a)*(b)*", "b"), ("(a)|(b)", "b"), ("(a)|(b)|(c)", "c"), ("((a)|(b))*", "ab"), ("((a)|(b))*", "ba"), ("(a(b)?)+", "aba"), ("(a(b)*)*", "abab"),
    # intervals
    ("a{2}", "aaa"), ("a{2,}", "aaaa"), ("a{1,2}", "aaa"), ("a{0,2}", "aaa"), ("a{,2}", "aaa"), ("a{0}", "a"), ("a{0}b", "ab"), ("a{0,0}b", "ab"),
    ("(a){0}b", "ab"), ("(a){2}", "aa"), ("(a){2,}", "aaa"), ("(a){1,3}", "aaaa"), ("(a|b){2}", "ab"), ("((a)|b){2}", "ab"), ("(a{2}){2}", "aaaaa"),
    ("(a*){2}", "aaa"), ("(a?){2}", "a"), ("(a?){3}", "aa"), ("(a|b){0,2}", "abab"), ("(^a){2}", "aa"), ("(a$){2}", "aa"), ("(\\<a){2}", "aa"), ("a{32767}", "a"),
    ("a{1}{2}", "aaa"), ("a{2}*", "aaaaa"), ("a*{2}", "aaa"), ("(a){2}\\1", "aaa"), ("(a|b){2}\\1", "abb"),
    # bad intervals
    ("a{", "a"), ("a{1", "a"), ("a{1,", "a"), ("a{1,2", "a"), ("a{}", "a"), ("a{,}", "aaa"), ("a{x}", "a"), ("a{1x}", "a"), ("a{1,x}", "a"), ("a{2,1}", "a"),
    ("a{1,2,3}", "a"), ("a{32768}", "a"), ("a{32768,}", "a"), ("a{1,32768}", "a"), ("a{99999999999}", "a"), ("{1}", "a"), ("a|{1}", "a"), ("({1})", "a"), ("a}", "a}"),
    ("}", "}"), ("a{1}}", "a}"), ("a{\\1}", "a"), ("a{1\\,2}", "a"), ("a{1,2\\}", "a"), ("(a){1\\,2}", "a"),
    # operators in the wrong place
    ("*", "a"), ("+", "a"), ("?", "a"), ("*a", "a"), ("(*a)", "a"), ("(+a)", "a"), ("a|*", "a"), ("a|+b", "a"), ("^*", "a"), ("^+", "a"), ("$*", "a"), ("(^*)", "a"),
    ("\\<*", "a"), ("\\b*", "a"), ("\\`*", "a"), ("\\'*", "a"),
    # groups
    ("(", "a"), (")", ")"), ("a)", "a)"), ("(a", "a"), ("(a))", "a)"), ("((a)", "a"), (")(", ")("), ("a(", "a"), ("(|", "a"), ("(a|", "a"), ("\\(", "("), ("\\)", ")"), ("\\(a\\)", "(a)"),
    # anchors
    ("^", ""), ("^", "a"), ("$", ""), ("$", "a"), ("^$", ""), ("^$", "a"), ("^a", "a"), ("^a", "ba"), ("a$", "a"), ("a$", "ab"), ("a^", "a"), ("$a", "a"), ("a^b", "ab"), ("a$b", "ab"),
    ("(^a)", "a"), ("(a$)", "a"), ("(^)a", "a"), ("a($)", "a"), ("^^a", "a"), ("a$$", "a"), ("(^|b)a", "a"), ("(^|b)a", "ba"), ("a($|b)", "ab"), ("a($|b)", "a"), ("^a|b$", "b"), ("^a|^b", "b"),
    ("(^a|^b)*", "ab"), ("(a|^)*", "aa"), ("x*^a", "a"), ("x*^a", "xa"), ("a$x*", "a"), ("a$x*", "ax"), ("^*a", "a"), ("\\`a", "a"), ("\\`a", "ba"), ("a\\'", "a"), ("a\\'", "ab"), ("\\`", ""), ("\\'", ""),
    ("(\\`)a", "a"), ("a(\\')", "a"), ("a\\`", "a"), ("\\'a", "a"), ("^\\`a", "a"), ("a$\\'", "a"), ("(^a){1}", "a"), ("(^a)*", "aa"), ("(a$)*", "aa"), ("(^a)+b", "ab"), ("(^)*", ""), ("(^)*a", "a"), ("($)*", ""),
    ("a\n^b", "a\nb"), ("a$\nb", "a\nb"), ("a\nb", "a\nb"), ("^b", "a\nb"), ("a$", "a\nb"), (".*", "a\nb"), ("a.b", "a\nb"), ("[^x]*", "a\nb"), ("(a\n)^b", "a\nb"),
    # word operators
    ("\\w", "a"), ("\\w", "_"), ("\\w", " "), ("\\W", " "), ("\\W", "a"), ("\\w+", " ab_1 "), ("\\W+", "a -b"), ("\\s", " "), ("\\s", "a"), ("\\S", " a"), ("\\s+", "a \t\nb"), ("\\S+", " ab "),
    ("\\b", ""), ("\\b", "a"), ("\\B", ""), ("\\B", "a"), ("\\B", "ab"), ("\\ba", "a"), ("a\\b", "a"), ("a\\b", "ab"), ("\\ba\\b", "b a b"), ("\\Ba", "ba"), ("a\\B", "ab"), ("a\\Bb", "ab"), ("a\\bb", "ab"), ("a\\b b", "a b"),
    ("\\<", ""), ("\\<", "a"), ("\\>", ""), ("\\>", "a"), ("\\<a", "ba a"), ("a\\>", "ab a"), ("\\<a\\>", "ab a"), ("a\\<", "a"), ("\\>a", "a"), ("(\\<a)", "a"), ("(a\\>)", "a"), ("\\<(a)", "a"), ("(a)\\>", "a"), ("\\<a*", "aa"), ("\\<a*\\>", "aa"),
    ("(\\b)a", "a"), ("a(\\b)", "a"), ("(\\ba|b)*", "ab"), ("(a\\b|b)*", "ab"), ("\\b\\b", "a"), ("\\b*", "a"), ("\\w\\b\\W", "a b"), ("\\W\\b\\w", " a"), ("_\\b", "_"), ("\\b_", "_"), ("[a]\\b", "a"), ("[^a]\\b", "ba"), (".\\b", "a"), (".\\b", "a b"), ("\\b.", " a"),
    # bracket expressions
    ("[a]", "a"), ("[a]", "b"), ("[^a]", "a"), ("[^a]", "b"), ("[abc]", "c"), ("[a-c]", "b"), ("[c-a]", "b"), ("[a-a]", "a"), ("[]]", "]"), ("[^]]", "]"), ("[^]]", "a"), ("[]a]", "a"), ("[a]]", "a]"), ("[-]", "-"), ("[a-]", "-"), ("[-a]", "-"),
    ("[a-c-e]", "d"), ("[--a]", "0"), ("[a--]", "a"), ("[%--]", "-"), ("[]-a]", "^"), ("[a-]]", "a]"), ("[^-a]", "b"), ("[^a-]", "b"), ("[[]", "["), ("[[a]", "["), ("[a[]", "["), ("[]", "a"), ("[^]", "a"), ("[", "a"), ("[a", "a"), ("[a-", "a"), ("[^", "a"), ("[]a", "a"), ("[^a", "a"), ("[\\]", "\\"), ("[\\a]", "a"), ("[\\]]", "\\]"),
    ("[.]", "."), ("[*]", "*"), ("[|]", "|"), ("[(]", "("), ("[)]", ")"), ("[{]", "{"), ("[$]", "$"), ("[a^]", "^"), ("[^^]", "^"), ("[^^]", "a"),
    ("[[:alpha:]]", "a"), ("[[:alpha:]]", "1"), ("[[:digit:]]", "1"), ("[[:alnum:]]", "_"), ("[[:upper:]]", "A"), ("[[:upper:]]", "a"), ("[[:lower:]]", "a"), ("[[:lower:]]", "A"), ("[[:space:]]", "\t"), ("[[:blank:]]", "\t"), ("[[:blank:]]", "\n"), ("[[:punct:]]", "!"), ("[[:print:]]", " "), ("[[:graph:]]", " "), ("[[:cntrl:]]", "\t"), ("[[:xdigit:]]", "f"), ("[[:xdigit:]]", "g"),
    ("[^[:alpha:]]", "a1"), ("[[:alpha:]0]", "0"), ("[[:foo:]]", "a"), ("[[:alpha:]", "a"), ("[[:alpha", "a"), ("[[:", "a"), ("[[:]", "a"), ("[[:]]", "a"), ("[[::]]", "a"), ("[:alpha:]", "a"), ("[[:alpha:]-z]", "-"), ("[a-[:alpha:]]", "a"), ("[[:alpha:]-]", "-"),
    ("[[.a.]]", "a"), ("[[.a.]-c]", "b"), ("[a-[.c.]]", "b"), ("[[.ab.]]", "a"), ("[[.-.]]", "-"), ("[[.].]]", "]"), ("[[..]]", "."), ("[[.", "a"), ("[[.a", "a"), ("[[.a.", "a"), ("[[.a.]", "a"), ("[[.space.]]", " "),
    ("[[=a=]]", "a"), ("[[=a=]-c]", "b"), ("[a-[=c=]]", "b"), ("[[=ab=]]", "a"), ("[[==]]", "="), ("[[=", "a"), ("[[=a=]", "a"), ("[[=]=]]", "]"), ("[[=a", "a"),
    ("[[:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:]]", "a"), ("[[.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.]]", "a"),
    # escapes
    ("\\", "a"), ("a\\", "a"), ("\\\\", "\\"), ("\\.", "."), ("\\.", "a"), ("\\*", "*"), ("\\+", "+"), ("\\?", "?"), ("\\|", "|"), ("\\{", "{"), ("\\}", "}"), ("\\[", "["), ("\\]", "]"), ("\\^", "^"), ("\\$", "$"), ("\\a", "a"), ("\\z", "z"), ("\\-", "-"), ("\\ ", " "), ("a\\|b", "a|b"), ("\\n", "n"), ("\\t", "t"), ("\\0", "0"), ("\\/", "/"), ("\\\n", "\n"),
    ("\\_", "_"), ("\\,", ","),
    # back-references
    ("(a)\\1", "aa"), ("(a)\\1", "ab"), ("(a*)\\1", "aaaa"), ("(a*)\\1", "aaaaa"), ("(a*)\\1\\1", "aaaaa"), ("(a|b)\\1", "abba"), ("(a|ab)\\1", "abab"), ("(.*)\\1", "abab"), ("(.*)\\1", "abcabc"), ("^(.*)\\1$", "abab"), ("^(.*)\\1$", "abcab"), ("(a)(b)\\2\\1", "abba"), ("(a)(b)\\1\\2", "abab"), ("((a)b)\\1\\2", "ababa"), ("(a)\\1*", "aaa"), ("(a)\\1+", "aaa"), ("(a)\\1?", "aa"), ("(a)\\1{2}", "aaaa"), ("(a*)*\\1", "aa"), ("(a*)+\\1", "aa"), ("()\\1", ""), ("()\\1*", ""), ("()\\1+", ""), ("()\\1*\\1*", ""), ("()\\1a", "a"), ("(a|)\\1", "aa"), ("(|a)\\1", "aa"), ("(a?)\\1", "aa"), ("(a?)\\1", "a"),
    ("\\1", "a"), ("\\1(a)", "aa"), ("(\\1)", "a"), ("(a\\1)", "aa"), ("(a)|\\1", "a"), ("(a)|b\\1", "b"), ("((a)|b)\\2", "aa"), ("((a)|b)\\2", "b"), ("(a)\\2", "a"), ("(a)(b)\\3", "ab"), ("(a)\\9", "a"), ("(a)(b)(c)(d)(e)(f)(g)(h)(i)\\9", "abcdefghii"), ("(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)\\10", "abcdefghijj"), ("(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)(k)", "abcdefghijk"),
    ("(a){0}\\1", "a"), ("(a){0}b\\1", "b"), ("(a)*\\1", "aaa"), ("(a)*\\1", ""), ("(a)?\\1", ""), ("(a)?\\1", "aa"), ("(a|b)*\\1", "abb"), ("(a|b)+\\1", "abab"), ("(^a)\\1", "aa"), ("(a$)\\1", "aa"), ("(a)\\1$", "aa"), ("^(a)\\1", "aa"), ("\\b(a)\\1\\b", "aa"), ("(\\ba)\\1", "aa"), ("(a\\b)\\1", "a a"), ("(a)\\b\\1", "aa"), ("(.)\\1", "abccd"), ("(.)(.)\\2\\1", "xabbay"), ("(..)\\1", "xabab"), ("(a+)b\\1", "aabaa"), ("(a+)b\\1", "aabaaa"), ("(a+)(b+)\\1\\2", "aabbaabb"), ("(a*)(b*)\\1\\2", "abab"), ("(a*)b\\1", "aab"),
    ("((a)\\2)", "aa"), ("((a)\\2)\\1", "aaaa"), ("(a(b)\\2)\\1", "abbabb"), ("((a*)\\2)*", "aaaa"), ("(a)(\\1)", "aa"), ("(a)(\\1)\\2", "aaa"), ("(a\\b|a)\\1", "aa"), ("(a|a\\b)\\1", "aa"), ("([ab])\\1", "abba"), ("[ab]*(a)\\1", "abaa"), ("(.*)(.*)\\1\\2", "abab"), ("(.*)b\\1", "abab"), ("(.*)b\\1", "aba"), ("(.?).\\1", "aba"), ("(.?)(.?)\\2\\1", "abba"), ("x(.*)y\\1z", "xaybz"), ("x(.*)y\\1z", "xayaz"), ("(a|b)(c|d)\\2\\1", "acca"), ("(a)(b)?\\2", "a"), ("(a)(b)?\\2", "abb"), ("(a)|(b)\\1", "b"),
    # subcount
    (("(a)(b)(c)", "abc", 0, 0)), (("(a)(b)(c)", "abc", 0, 1)), (("(a)(b)(c)", "abc", 0, 2)), (("(a)(b)(c)", "abc", 0, 3)), (("(a)(b)(c)", "abc", 0, 4)), (("(a)(b)(c)", "abc", 0, 5)), (("(a)(b)(c)", "abc", 0, 100)), (("a", "a", 0, 0)), (("a", "b", 0, 0)), (("a", "a", 0, 1)), (("a", "a", 0, 2)), (("(a)\\1", "aa", 0, 0)), (("(a)\\1", "aa", 0, 1)), (("(a)\\1", "aa", 0, 2)), (("(a)\\1", "ab", 0, 1)), (("(a)(b)\\2", "abb", 0, 2)), (("(a)(b)\\1", "aba", 0, 2)), (("(a*)*", "aa", 0, 1)), (("(a|b)*", "ab", 0, 2)),
    # icase
    (("a", "A", 1, None)), (("A", "a", 1, None)), (("a", "a", 1, None)), (("A", "A", 1, None)), (("abc", "aBC", 1, None)), (("[a]", "A", 1, None)), (("[A]", "a", 1, None)), (("[^a]", "A", 1, None)), (("[^a]", "b", 1, None)), (("[a-c]", "B", 1, None)), (("[A-C]", "b", 1, None)), (("[_-a]", "_", 1, None)), (("[_-a]", "_", 0, None)), (("[Z-a]", "_", 0, None)), (("[Z-a]", "_", 1, None)), (("[[:lower:]]", "A", 1, None)), (("[[:upper:]]", "a", 1, None)), (("[[:alpha:]]", "A", 1, None)), (("[[:LOWER:]]", "A", 1, None)), (("[[:ALPHA:]]", "A", 0, None)), (("[[.a.]]", "A", 1, None)), (("[[=a=]]", "A", 1, None)), (("[[.A.]]", "a", 1, None)), (("[[.a.]-c]", "B", 1, None)), (("\\a", "a", 1, None)), (("\\a", "A", 1, None)), (("\\A", "a", 1, None)), (("\\A", "A", 1, None)), (("\\w", "A", 1, None)), (("\\W", "A", 1, None)), (("\\s", " ", 1, None)), (("\\S", "a", 1, None)), (("\\b", "a", 1, None)), (("\\B", "a", 1, None)), (("(a)\\1", "aA", 1, None)), (("(A)\\1", "Aa", 1, None)), (("(.)\\1", "aA", 1, None)), (("(.)\\1", "Ab", 1, None)), (("a{2}", "aA", 1, None)), (("^a$", "A", 1, None)), (("\\<a\\>", "A", 1, None)), (("a|B", "b", 1, None)), (("(a|B)*", "AbB", 1, None)), (("\xe4", "\xc4", 1, None)), (("\xc4", "\xe4", 1, None)), (("[\xe4]", "\xc4", 1, None)), (("z", "Z", 1, None)), (("[y-z]", "Z", 1, None)), (("[@-\\[]", "a", 1, None)), (("[`-a]", "a", 1, None)), (("[`-a]", "a", 0, None)),
    # bytes above 127
    ("\xe4", "\xe4"), ("\xe4", "a"), (".", "\xe4"), ("[\xe4]", "\xe4"), ("[^\xe4]", "\xe4"), ("[\x80-\xff]", "\xc3"), ("[\xff-\x80]", "\xc3"), ("\\w", "\xe4"), ("\\W", "\xe4"), ("\\s", "\xa0"), ("\\S", "\xa0"), ("[[:alpha:]]", "\xe4"), ("[[:alnum:]]", "\xe4"), ("[[:print:]]", "\xe4"), ("[[:graph:]]", "\xe4"), ("[[:punct:]]", "\xe4"), ("[[:cntrl:]]", "\x80"), ("[[:space:]]", "\x85"), ("[[:upper:]]", "\xc4"), ("[[:lower:]]", "\xe4"), ("\\b", "\xe4"), ("\xe4\\b", "\xe4"), ("(.)\\1", "\xc3\xc3"), ("\xc3\xa4", "\xc3\xa4"), (".", "\xc3\xa4"), ("..", "\xc3\xa4"), ("^.$", "\xc3\xa4"), ("\\\xe4", "\xe4"),
    # positions
    ("b", "aaab"), ("(b)", "aaab"), ("(b)|(c)", "aaac"), ("b*", "aaab"), ("(b*)", "aaab"), ("(a)(b)?", "cab"), ("(a)(b)?", "ca"), ("x*", "aaxx"), ("(x*)y", "axxy"), ("abc|b", "ab"), ("(abc|b)", "ab"), ("(ab)*c", "ababx abc"), ("(a|b)*c|(a|ab)*c", "abc"), ("(a|b)*c|(a|ab)*c", "xc"), ("(.a|.b).*|.*(.a|.b)", "xa"), ("a(b)|c(d)|a(e)f", "aef"), ("(a|b)?.*", "b"), ("(a|b)c|a(b|c)", "ac"), ("(a|b)c|a(b|c)", "ab"), ("(a|b)*c|(a|ab)*c", "abc"), ("(.a|.b).*|.*(.a|.b)", "xa"),
    # classic tests of submatches
    ("(a|ab)(c|bcd)(d*)", "abcd"), ("(wee|week)(knights|night)", "weeknights"), ("(a*)(ab)*(b*)", "aabb"), ("(a*)(ab)*(b*)", "abab"), ("(foo|foobar)(bar)?", "foobar"), ("(a)|b|c(d)|e", "e"), ("(.*)c(.*)", "abcde"), ("a(bc)d", "abcd"), ("a[-]?c", "ac"), ("(((((((((a)))))))))", "a"), ("(ab)c|abc", "abc"), ("a{0}b", "ab"), ("(a*)(b{0,1})(b{1,})b{3}", "aaabbbbbbb"), ("(a*)(b{0,1})(b{1,})b{3}", "bbbbbbb"), ("((..)|(.))((..)|(.))((..)|(.))", "aaa"), ("((..)|(.)){3}", "aaa"), ("((..)|(.))*", "aaa"), ("(a|b)*(a|b)", "abab"), ("(.)*(.)", "ab"), ("(a|aa)*", "aaa"), ("(aa|a)*", "aaa"), ("(a|aa)+", "aaaa"), ("(a?)((ab)?)", "ab"), ("(a?)((ab)?)(b?)", "ab"), ("((a?)((ab)?))(b?)", "ab"), ("(a?)(((ab)?)(b?))", "ab"), ("(.?)", "x"), ("(.?){1}", "x"), ("(.?)(.?)", "x"), ("(.?){2}", "x"), ("(.?)*", "x"), ("(.?.?)", "xxx"), ("(.?.?){1}", "xxx"), ("(.?.?)(.?.?)", "xxx"), ("(.?.?){2}", "xxx"), ("(.?.?)(.?.?)(.?.?)", "xxx"), ("(.?.?){3}", "xxx"), ("(.?.?)*", "xxx"), ("a?((ab)?)(b?)", "ab"), ("(a?)((ab)?)b?", "ab"), ("a?((ab)?)b?", "ab"), ("(a*)*", "a"), ("(a*)*", "x"), ("(a*)*", "aaaaaa"), ("(a*)*", "aaaaaax"), ("(a*)+", "a"), ("(a*)+", "x"), ("(a*)+", "aaaaaa"), ("(a*)+", "aaaaaax"), ("(a+)*", "a"), ("(a+)*", "x"), ("(a+)*", "aaaaaa"), ("(a+)*", "aaaaaax"), ("(a+)+", "a"), ("(a+)+", "x"), ("(a+)+", "aaaaaa"), ("(a+)+", "aaaaaax"), ("([a]*)*", "a"), ("([a]*)*", "x"), ("([a]*)*", "aaaaaa"), ("([a]*)*", "aaaaaax"), ("([a]*)+", "a"), ("([a]+)*", "aaaaaax"), ("([^b]*)*", "a"), ("([^b]*)*", "b"), ("([^b]*)*", "aaaaaa"), ("([^b]*)*", "aaaaaab"), ("([ab]*)*", "a"), ("([ab]*)*", "aaaaaa"), ("([ab]*)*", "ababab"), ("([ab]*)*", "bababa"), ("([ab]*)*", "b"), ("([ab]*)*", "bbbbbb"), ("([ab]*)*", "aaaabcde"), ("([^a]*)*", "b"), ("([^a]*)*", "bbbbbb"), ("([^a]*)*", "aaaaaa"), ("([^ab]*)*", "ccccxx"), ("([^ab]*)*", "ababab"), ("((z)+|a)*", "zabcde"), ("(a)", "aaa"), ("(a*)*(x)", "x"), ("(a*)*(x)", "ax"), ("(a*)*(x)", "axa"), ("(a*)+(x)", "x"), ("(a*)+(x)", "ax"), ("(a*)+(x)", "axa"), ("(a*){2}(x)", "x"), ("(a*){2}(x)", "ax"), ("(a*){2}(x)", "axa"),
    ("(aba|ab|a)*", "ababa"), ("(a|ab|aba)*", "ababa"), ("(aba|ab|a)(aba|ab|a)(aba|ab|a)", "ababa"), ("(a|b)*(ab)*", "abab"), ("(a*)*(a)", "aa"), ("(a*)(a*)(a*)", "aa"), ("(a|ab)(bc|c)?", "abc"), ("(a)(b)(c)|(d)", "d"), ("x(y|())*z", "xyyz"), ("x(()|y)*z", "xyyz"), ("(x|)*y", "xxy"), ("(|x)*y", "xxy"), ("(x|){2}y", "xy"), ("(|x){2}y", "xy"), ("(x?|y)*", "xy"), ("(y|x?)*", "xy"), ("((x)|(y))+", "xy"), ("((x)|(y)){2}", "xy"), ("((x)|y)*z", "xyz"), ("(x|(y))*z", "yxz"), ("(a(b)|c)*", "abc"), ("(c|a(b))*", "abc"), ("(a(b)?)*", "aba"), ("(a(b*))*", "aba"), ("(a(b*))+", "aba"), ("(a(b)*)+", "aba"), ("((a)|b)*", "ab"), ("((a)|b)+", "ab"), ("((a)|b){2}", "ab"), ("((a)*)*", "aa"), ("((a)+)+", "aa"), ("((a)?)*", "aa"), ("((a*)*)*", "aa"), ("((a*)+)*", "aa"), ("((a+)*)+", "aa"),
]


def to_cases():
    out = []
    for c in HAND:
        if len(c) == 2:
            out.append((c[0], c[1], 0, None))
        else:
            out.append(c)
    return out


def random_pattern(r, depth=0):
    n = r.choice((1, 1, 2, 2, 3))
    parts = []
    for _ in range(n):
        parts.append(random_branch(r, depth))
    return "|".join(parts) if r.random() < 0.25 else parts[0]


def random_branch(r, depth):
    s = ""
    for _ in range(r.randint(0 if r.random() < 0.1 else 1, 4)):
        s += random_piece(r, depth)
    return s


def random_piece(r, depth):
    atom = random_atom(r, depth)
    x = r.random()
    if x < 0.55:
        return atom
    return atom + r.choice(("*", "*", "*", "+", "+", "?", "?", "{2}", "{1,2}", "{0,2}", "{2,}", "{,1}", "{0}", "**", "*?", "+*"))


def random_atom(r, depth):
    x = r.random()
    if x < 0.34 or depth > 3:
        return r.choice("aaabbc")
    if x < 0.40:
        return "."
    if x < 0.47:
        return r.choice(("[ab]", "[^a]", "[a-c]", "[^b-c]", "\\w", "\\W", "\\s", "\\S", "[[:alpha:]]", "[_a]", "[ ]"))
    if x < 0.55:
        return r.choice(("^", "$", "\\b", "\\B", "\\<", "\\>", "\\`", "\\'"))
    if x < 0.60:
        return r.choice(("()", "(|a)", "(a|)", "(^)", "($)", "\\1", "\\2", "\\1", "_", " "))
    return "(" + random_pattern(r, depth + 1) + ")"


def random_text(r):
    return "".join(r.choice("aaabbbc _A") for _ in range(r.randint(0, 8)))


def hexof(s):
    return s.encode("latin-1").hex()


def run_oracle(cases, tmp):
    lines = [PREAMBLE]
    for k, (p, t, ic, sc) in enumerate(cases):
        lines.append("\\case{%d}{%s}{%s}{%s}{%d}" % (k, hexof(p), hexof(t), "icase " if ic else "", 10 if sc is None else sc))
    lines.append("\\csname bye\\endcsname\\end")
    path = os.path.join(tmp, "t.tex")
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")
    env = dict(os.environ, max_print_line="1000000", error_line="254", half_error_line="238")
    try:
        p = subprocess.run(["pdftex", "-ini", "-etex", "-interaction=batchmode", "t.tex"], cwd=tmp, env=env, timeout=20,
                           stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    except subprocess.TimeoutExpired:
        return None  # a pattern on which glibc's regexec runs for ever
    if p.returncode < 0:
        return None  # a pattern that crashes glibc's regexec: the random cases can find one
    with open(os.path.join(tmp, "t.log"), "rb") as f:
        return f.read().decode("latin-1")


def run_cases(cases, tmp):
    """The cases that the oracle survives, and their rows."""
    log = run_oracle(cases, tmp)
    if log is not None:
        return cases, parse_log(log, cases)
    if len(cases) == 1:
        print("oracle crashes on", repr(cases[0]))
        return [], []
    half = len(cases) // 2
    c1, r1 = run_cases(cases[:half], tmp)
    c2, r2 = run_cases(cases[half:], tmp)
    return c1 + c2, r1 + r2


def field(s):
    out = []
    for c in s:
        o = ord(c)
        if c == "%" or o < 32 or o >= 127:
            out.append("%%%02X" % o)
        else:
            out.append(c)
    return "".join(out)


def parse_log(log, cases):
    rows = []
    parts = re.split(r"^CASE:(\d+):$", log, flags=re.M)
    if (len(parts) - 1) // 2 != len(cases):
        sys.exit("regex: %d results for %d cases" % ((len(parts) - 1) // 2, len(cases)))
    for k, case in enumerate(cases):
        assert int(parts[2 * k + 1]) == k
        text = parts[2 * k + 2]
        m = re.search(r"^RES:%d:(.*):$" % k, text, flags=re.M)
        if not m:
            sys.exit("regex: no result for case %d %r" % (k, case))
        res = m.group(1)
        if res == "-1":
            w = re.search(r"pdfTeX warning: [a-z]+: \\pdfmatch: (.*)$", text, flags=re.M)
            res = w.group(1) if w else "?"
        lm = [bytes.fromhex(x).decode("latin-1") for x in re.findall(r"^LM:%d:\d+:([0-9A-F]*):$" % k, text, flags=re.M)]
        if len(lm) != 12:
            sys.exit("regex: %d lastmatch lines for case %d %r" % (len(lm), k, case))
        rows.append((res, lm))
    return rows


def main():
    n_random, seed, out, refs = 1000, 20261102, OUT, 0.0
    args = sys.argv[1:]
    while args:
        a = args.pop(0)
        if a == "--random":
            n_random = int(args.pop(0))
        elif a == "--seed":
            seed = int(args.pop(0))
        elif a == "--refs":
            refs = float(args.pop(0))
        elif a == "--out":
            out = args.pop(0)
        else:
            sys.exit("unknown argument " + a)
    r = random.Random(seed)
    cases = to_cases()
    for _ in range(n_random):
        ic = 1 if r.random() < 0.1 else 0
        sc = None if r.random() < 0.9 else r.randint(0, 4)
        p = random_pattern(r)
        if r.random() < refs:
            p = "(%s)%s\\1%s" % (r.choice(("a*", "a|b", ".", ".*", "a?", "b+", "[ab]*", "(a)|b")), r.choice(("", "b", ".", "a*")), r.choice(("", "*", "+", "?", "$")))
        cases.append((p, random_text(r), ic, sc))
    kept, rows = [], []
    for start in range(0, len(cases), 1000):
        with tempfile.TemporaryDirectory() as tmp:
            c, rs = run_cases(cases[start:start + 1000], tmp)
            kept.extend(c)
            rows.extend(rs)
    with open(out, "wb") as f:
        for (p, t, ic, sc), (res, lm) in zip(kept, rows):
            while lm and lm[-1] == "-1->":
                lm = lm[:-1]
            fields = [field(p), field(t), str(ic), "-" if sc is None else str(sc), field(res)] + [field(x) for x in lm]
            f.write(("\t".join(fields) + "\n").encode("latin-1"))
    print("regex:", len(kept), "cases in", out)


if __name__ == "__main__":
    main()

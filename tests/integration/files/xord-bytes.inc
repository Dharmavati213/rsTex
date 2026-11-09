% Every byte through input_ln and xord, from a file this writes and from the terminal.  run.py's COMMAND_LINES run it as section 24 leaves xord, with -8bit, with a TCX file,
% in a format, whose xord comes from the file, and in a format with a TCX file, which keeps the run's.  The file has each byte as \write prints it: as it is where xprn says so, else in ^^ notation.
\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7
\tracingonline=1 \newlinechar=-1
% \z: each character code once, as a letter; \lowercase cannot make ^^@.  \s: three spaces, which input_ln takes off the end of a line.
{\catcode0=11 \gdef\z{^^@}}
\count1=1
\def\more{\lccode`\A=\count1 \lowercase{\edef\z{\z A}}\advance\count1 by 1 \ifnum\count1<256 \expandafter\more\fi}
\more
\lccode`\A=32 \lowercase{\edef\s{AAA}}
\immediate\openout1=xord-out.tex
\immediate\write1{\z}
\immediate\write1{\z\s}
\immediate\write1{\s}
\immediate\closeout1
% A line read with every code of category 12 and no end of line: \meaning shows the bytes it holds.
\def\allother{\catcode\count1=12 \advance\count1 by 1 \ifnum\count1<256 \expandafter\allother\fi}
\def\readline#1{\begingroup\count1=0 \allother\endlinechar=-1 \read#1 to\x\global\let\y\x\endgroup\message{[\meaning\y]}}
\def\readfile{\ifeof1 \else\readline1\expandafter\readfile\fi}
\openin1=xord-out \readfile
% The four lines run.py gives on standard input.
\readline{16}\readline{16}\readline{16}\readline{16}
\end

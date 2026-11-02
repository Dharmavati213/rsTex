% Input by the numbers_d* cases, which differ in \pdfdecimaldigits.  One page of rules placed so that both coordinates
% go through pdf_print_bp with every sign and at values around the rounding points of divide_scaled, and widths that
% give pdf_print_real a zero fraction, leading zeros and trailing zeros.  \mark goes to the log so that a mismatch can
% be found by value.
\def\R#1#2{% a rule at x=#1 sp, y=#2 sp from the top, width and height big enough for the `re' form
  \hbox to 0pt{\kern#1sp\raise-#2sp\hbox{\vrule width 5pt height 5pt depth 0pt}\hss}\kern-\ht0}
\setbox0\hbox{}
\def\W#1{\hbox to 0pt{\vrule width #1sp height 5pt depth 0pt\hss}}
\setbox2\vbox to 0pt{\baselineskip=0pt \lineskiplimit=-1000pt \lineskip=0pt
  \R{0}{1000000}\R{1}{1000000}\R{32}{1000000}\R{33}{1000000}\R{65}{1000000}\R{66}{1000000}\R{657}{1000000}\R{658}{1000000}
  \R{-1}{1000000}\R{-33}{1000000}\R{-65}{1000000}\R{-66}{1000000}\R{-657}{1000000}\R{-658}{1000000}\R{-65782}{1000000}
  \R{65536}{1000000}\R{65781}{1000000}\R{65782}{1000000}\R{65783}{1000000}\R{657817}{1000000}\R{657818}{1000000}
  \R{6578176}{1000000}\R{6578175}{1000000}\R{6578177}{1000000}\R{-6578176}{1000000}\R{98672640}{1000000}\R{1073741823}{1000000}
  \R{-1073741823}{1000000}\R{1000000}{0}\R{1000000}{1}\R{1000000}{33}\R{1000000}{-1}\R{1000000}{-33}\R{1000000}{-1000000}
  \W{65783}\W{65784}\W{65792}\W{66000}\W{72270}\W{131564}\W{328908}\W{657817}\W{657818}\W{6578176}\W{65781760}
  \W{1073741823}\W{100000}\W{100001}\W{110000}\W{101010}\W{500000}\W{3333333}\W{6666666}\W{7654321}
  \vss}
\wd2=0pt \ht2=0pt \dp2=0pt
\shipout\box2
\end

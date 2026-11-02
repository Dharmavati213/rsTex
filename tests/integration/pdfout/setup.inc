% What the pdfout and pdfobj cases share: PDF output with nothing compressed, so that the log's byte count and the
% objects can be compared directly, and nothing that depends on the date or on the output path.
\pdfoutput=1 \pdfcompresslevel=0 \pdfinfoomitdate=1 \pdfsuppressptexinfo=-1 \pdftrailerid{}
\pdfpagewidth=400pt \pdfpageheight=400pt \pdfhorigin=0pt \pdfvorigin=0pt
\voffset=0pt \hoffset=0pt
\def\nullbox{\hbox{\vrule width 20pt height 10pt depth 0pt}}
% plain's \loop
\def\loop#1\repeat{\def\body{#1}\iterate}
\def\iterate{\body \let\next\iterate \else\let\next\relax\fi \next}
\let\repeat=\fi
% A paragraph of its own as one line: hpack's cal_expand_ratio happens only for the lines of a paragraph, not for \hbox to.
\def\line#1#2{{\hsize=#1\parfillskip=0pt \parindent=0pt \leftskip=0pt \rightskip=0pt \emergencystretch=0pt \noindent #2\par}}

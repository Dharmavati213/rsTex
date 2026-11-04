"""An adversarial public document that holds as much state as it can across page boundaries."""
import os, shutil
TEX = r'''\documentclass{article}
\usepackage[T1]{fontenc}
\usepackage{graphicx}
\usepackage{xcolor}
\usepackage{microtype}
\usepackage[pdfusetitle]{hyperref}
\newwrite\wa \newread\ra \newread\rb
\pdfgentounicode=1
\newcommand\filler[1]{\par #1 The quick brown fox jumps over the lazy dog and then it goes on for some time, with $x^2+y^2=z^2$ inline.
What the dog thought of it all is a question for another day and another document entirely.\par}
\begin{document}
\tableofcontents
\clearpage
\section{Match and streams}
\label{sec:one}
\pdfmatch{b+}{aabbcc}
\immediate\openout\wa=side.out
\immediate\write\wa{\string\def\string\fromside{immediate one}}
\write\wa{\string\def\string\fromdeferred{deferred \thepage}}
\openin\ra=data.txt
\read\ra to\linea \linea
\color{red}
\filler{Red starts.}
\clearpage
\section{Map line and glyph unicode}
\pdfmapline{=cmss10 CMSS10 <cmss10.pfb}
\pdfmapline{+pcrr8r Courier " TeXBase1Encoding ReEncodeFont " <8r.enc <ucrr8a.pfb}
\pdfglyphtounicode{a}{0061}
{\fontencoding{OT1}\fontfamily{cmss}\selectfont sans text here \filler{Sans.}}
\read\ra to\lineb \lineb
\clearpage
\section{PNG}
\pdfximage width 3cm {example-image.png}\xdef\pngref{\the\pdflastximage}
\pdfrefximage\pngref\relax
\pdfximage width 2cm {alpha.png}\xdef\alpharef{\the\pdflastximage}
\filler{Colour goes on.}
\clearpage
\section{JPEG and reuse}
\pdfximage width 2cm {example-image.jpg}\xdef\jpgref{\the\pdflastximage}
\pdfrefximage\jpgref\relax \pdfrefximage\pngref\relax \pdfrefximage\alpharef\relax
\normalcolor
\clearpage
\section{Last match, sums}
\pdflastmatch 0 \ifnum\pdfshellescape=0 none\fi
\pdfmdfivesum file {data2.txt} and \pdffilesize{data3.txt} and \pdffilemoddate{data3.txt} and \pdffiledump offset 0 length 4 {data2.txt}
\filler{Sums.}
\clearpage
\section{PDF figure}
\includegraphics[width=3cm]{example-image.pdf}
\filler{After a figure.}
\clearpage
\section{After the PDF figure}
\filler{Later.}
\includegraphics[width=2cm,page=1]{example-image.pdf}
\clearpage
\section{Forms, objects, links over a page break}
\setbox0\hbox{form text \textbf{bold}}\pdfxform 0\xdef\formref{\the\pdflastxform}
\pdfobj stream {a stream object}\pdfrefobj\pdflastobj
\pdfobj file {data2.txt}\pdfrefobj\pdflastobj
\pdfobj reserveobjnum \xdef\resobj{\the\pdflastobj}
\pdfrefxform\formref\relax
\pdfoutline user{/S/URI/URI(http://example.org)} count 0 {An outline}
\pdfdest name {mydest} xyz
\leavevmode\pdfstartlink attr{/Border[0 0 0]} goto name {mydest}\relax linked text\pdfendlink
\filler{A link.}
\leavevmode\pdfsavepos\the\pdflastxpos\ \the\pdflastypos
\clearpage
\pdfobj useobjnum \resobj {(filled later)}
\section{Inside scantokens and an input file}
\begingroup\everyeof{\noexpand}\endlinechar=-1 \scantokens{\filler{in scantokens}\clearpage \filler{still in scantokens}}\endgroup
\input{sub}
\filler{Back from sub.}
\clearpage
\section{Fonts made in the body}
\font\xx=cmr12 at 13pt \xx some text at 13 \normalfont
\letterspacefont\yy\xx 100 \yy spaced \normalfont
\pdfcopyfont\zz\xx \zz copied \normalfont
\pdfincludechars\xx{QXZ}
\pdffontattr\xx{/MyAttr true}
\pdfadjustspacing=2 \pdfprotrudechars=2
\filler{Expansion on.}
\clearpage
\section{Clock and lookups}
\the\year-\the\month-\the\day\ \today
\IfFileExists{absent.tex}{\input{absent}}{not there}
\IfFileExists{shadow.tex}{\input{shadow}}{not there}
\filler{Clock.}
\read\ra to\linec \linec
\clearpage
\section{Closing}
\closeout\wa
\closein\ra
\filler{Closed.}
\clearpage
\section{Read back what was written}
\input{side.out}
\fromside
\ifdefined\fromdeferred\fromdeferred\else no deferred\fi
\pageref{sec:one}
\filler{Read back.}
\pdfinfo{/Keywords (adversarial)}
\pdfcatalog{/PageMode /UseOutlines}
\pdfnames{/MyName (x)}
\pdftrailer{/MyTrailer true}
\pdfpageattr{/Rotate 0}
\clearpage
\filler{The last page.}
\end{document}
'''
def png_alpha(path, w=8, h=8):
    import zlib, struct
    def ch(t, d):
        c = struct.pack('>I', len(d)) + t + d
        return c + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    raw = b''.join(b'\0' + bytes((x * 30 % 256, y * 30 % 256, 128, 200) for x in range(w) for _ in ()) if False else b'\0' + b''.join(bytes((x*30%256, y*30%256, 128, 200)) for x in range(w)) for y in range(h))
    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b''))

def write(src, env_note=None):
    os.makedirs(src, exist_ok=True)
    open(os.path.join(src, 'adv.tex'), 'w').write(TEX)
    open(os.path.join(src, 'sub.tex'), 'w').write('\\filler{In sub, first.}\n\\clearpage\n\\filler{In sub, second.}\n\\clearpage\n\\filler{In sub, third.}\n\\endinput\nThis line is never read.\n')
    open(os.path.join(src, 'data.txt'), 'w').write('line one of the data\n\nline three of the data\n\nline five of the data\n\nline seven\n')
    open(os.path.join(src, 'data2.txt'), 'w').write('data two contents\n')
    open(os.path.join(src, 'data3.txt'), 'w').write('data three contents\n')
    png_alpha(os.path.join(src, 'alpha.png'))
    for n in ('example-image.png', 'example-image.jpg', 'example-image.pdf'):
        shutil.copy('/usr/share/texmf-dist/tex/latex/mwe/' + n, os.path.join(src, n))

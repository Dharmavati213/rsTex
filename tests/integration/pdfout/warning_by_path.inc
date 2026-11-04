% Run by image/cli:warning_by_path, with argv[0] the program's whole path: pdftex_warn prints kpse_invocation_name, and that is argv[0] as it came.
\catcode`\{=1 \catcode`\}=2
\pdfoutput=1 \pdfcompresslevel=0 \pdfobjcompresslevel=0 \pdfinfoomitdate=1 \pdftrailerid{}
\pdfmapline{+cmr10 NameOne <nosuchfile.pfb}
\pdfmapline{+cmr10 NameTwo <nosuchfile.pfb}
\font\a=cmr10
\setbox1\hbox{\a abc}
\shipout\box1
\end

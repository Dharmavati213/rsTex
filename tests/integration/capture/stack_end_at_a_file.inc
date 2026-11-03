% A file level that the input stack has no room for: section 350 raises in_open before it pushes, so the context of the overflow reads a line_stack entry never set.
\catcode`\{=1 \catcode`\}=2 \catcode`\%=14
\def\r{\advance\count1 -1 \ifnum\count1>0 \r{}\else\snippet\relax}\def\snippet{\scantokens{}}\count1=198
\r
\end

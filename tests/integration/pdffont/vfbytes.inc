% Writes a file of raw bytes: \vffile{name}{bytes}.  Under the plain format's table of printable characters, byte 9
% and bytes 33 to 126 and 128 to 255 reach the file as they are.  A virtual font must fit that: no zeros, so no real
% fix_words, and a name of 9 characters or at least 33.  What this makes is therefore always a little wrong, which
% is the point of most cases.
\catcode`\|=0 \catcode`\[=1 \catcode`\]=2 \catcode9=12 \catcode`\~=12 \catcode`\%=12 \catcode`\&=12 \catcode`\_=12
\catcode`\$=12 \catcode`\{=12 \catcode`\}=12 \catcode`\\=12
|newwrite|vfout
|def|vffile#1#2[|immediate|openout|vfout=#1.vf |immediate|write|vfout[#2]|immediate|closeout|vfout]

% oracle-compat.ch: applied last when making refs/merged/pdftex-merged.web.
%
% The oracle (pdfTeX 1.40.29, TeX Live 2026/Arch) is built with enctex1.ch,
% enctex-pdftex.ch and enctex2.ch.  rstex omits those three files.  Reviewing
% the 800-line difference shows that with encTeX switched off (no -enc, which
% is how every format here is built: \mubytein=\mubyteout=\mubytelog=\specialout=0)
% every path is behaviourally the same as without the change files, with one
% exception, which lives in issue_message and print:
%
%   Without encTeX, \message and \errmessage build the string raw (selector
%   new_string > pseudo) and slow_print it, so unprintable characters become
%   ^^-notation only when printed.  With encTeX compiled in, message_printing
%   makes print expand ^^-notation while the string is being built, and the
%   result is then printed with print.  The difference is observable: the test
%   term_offset+length(s)>max_print_line-2 counts the expanded length.
%
% This change file restores the oracle's path, keeping only the single global
% that has an effect when encTeX is off.  The xprn test is the oracle's too.

@x [2.20] oracle compat: global used by print and issue_message
xprn: array [ASCII_code] of ASCII_code;
   { non zero iff character is printable }
@y
xprn: array [ASCII_code] of ASCII_code;
   { non zero iff character is printable }
message_printing: boolean; { true if message or errmessage prints to string }
@z

@x [2.23] oracle compat: initialise it
for i:=@'177 to @'377 do xchr[i]:=i;
@y
for i:=@'177 to @'377 do xchr[i]:=i;
message_printing := false;
@z

@x [5.59] oracle compat: print path of \message and \errmessage
  else begin if selector>pseudo then
      begin print_char(s); return; {internal strings are not expanded}
      end;
    if (@<Character |s| is the current new-line character@>) then
      if selector<pseudo then
        begin print_ln; return;
        end;
@y
  else begin if (selector>pseudo) and (not message_printing) then
      begin print_char(s); return; {internal strings are not expanded}
      end;
    if (@<Character |s| is the current new-line character@>) then
      if selector<pseudo then
        begin print_ln; return;
        end
      else if message_printing then
        begin print_char(s); return;
        end;
    if xprn[s] then
      begin print_char(s); return; end;
@z

@x [49.1279] oracle compat: \message builds the expanded string
token_show(def_ref); selector:=old_setting;
flush_list(def_ref);
str_room(1); s:=make_string;
if c=0 then @<Print string |s| on the terminal@>
@y
message_printing := true;
token_show(def_ref);
message_printing := false;
selector:=old_setting;
flush_list(def_ref);
str_room(1); s:=make_string;
if c=0 then @<Print string |s| on the terminal@>
@z

@x [49.1280] oracle compat: and prints it with print
slow_print(s); update_terminal;
@y
print(s); update_terminal;
@z

@x [49.1283] oracle compat: same for \errmessage
begin print_err(""); slow_print(s);
@y
begin print_err(""); print(s);
@z

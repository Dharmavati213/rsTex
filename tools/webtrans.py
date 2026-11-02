"""Translate WEB @d macros to Rust accessor methods.

Handles macros whose bodies are expressions or simple statements over globals, consts, other
macros and procedures.  A macro using a local variable of a procedure, a goto, or `#` in a way that
cannot be a parameter is left for its owner and reported; see tools/webgen.py.
"""
import re


class NoTrans(Exception):
    pass


# Globals with names so short that a macro using them may well mean a local of its caller.
SHORT_GLOBALS = {"c", "f", "g", "k", "l", "delta", "ff"}
SHORT_GLOBAL_OK = {"app_lc_hex", "set_ff"}  # checked by hand: these really are the globals


TOK = re.compile(
    r"""\s*(?:
      (?P<oct>@'[0-7]+) | (?P<hex>@"[0-9A-Fa-f]+) | (?P<real>\d+\.\d+) | (?P<num>\d+)
    | "(?P<wstr>(?:[^"]|"")*)" | '(?P<pstr>(?:[^']|'')*)'
    | (?P<id>[A-Za-z_][A-Za-z_0-9]*)
    | (?P<op>:=|<>|<=|>=|\.\.|[-+*/=<>()\[\],.;:\#])
    )""",
    re.X,
)


def tokenize(s):
    s = re.sub(r"@[!?/|;,+\\]", " ", s)
    s = re.sub(r"@t.*?@>", " ", s, flags=re.S)
    s = re.sub(r"@\^.*?@>|@\..*?@>|@:.*?@>", " ", s, flags=re.S)
    out = []
    pos = 0
    n = len(s)
    while pos < n:
        m = TOK.match(s, pos)
        if not m:
            if s[pos:].strip() == "":
                break
            raise NoTrans("tokenize at %r" % s[pos : pos + 20])
        pos = m.end()
        k = m.lastgroup
        v = m.group(k)
        if k == "oct":
            out.append(("num", int(v[2:], 8)))
        elif k == "hex":
            out.append(("num", int(v[2:], 16)))
        elif k == "num":
            out.append(("num", int(v)))
        elif k == "real":
            out.append(("real", v))
        elif k == "wstr":
            st = v.replace('""', '"').replace("@@", "@")
            if len(st) == 1:
                out.append(("char", st))
            else:
                out.append(("str", st))
        elif k == "pstr":
            out.append(("pstr", v.replace("''", "'").replace("@@", "@")))
        elif k == "id":
            out.append(("id", v))
        else:
            out.append(("op", v))
    return out


def rust_bytes(s):
    o = []
    for ch in s:
        c = ord(ch)
        if ch == '"':
            o.append('\\"')
        elif ch == "\\":
            o.append("\\\\")
        elif 32 <= c < 127:
            o.append(ch)
        else:
            o.append("\\x%02x" % c)
    return 'b"' + "".join(o) + '"'


def char_lit(ch):
    if ch == "'":
        return "(b'\\'' as i32)"
    if ch == "\\":
        return "(b'\\\\' as i32)"
    return "(b'%s' as i32)" % ch


class E:
    """A translated expression."""

    __slots__ = ("code", "ty", "set", "mut", "lets", "place")

    def __init__(self, code, ty="i32", set=None, mut=False, lets=(), place=None):
        self.code = code
        self.ty = ty
        self.set = set  # function(value code) -> statement code, for an lvalue
        self.mut = mut  # needs &mut self
        self.lets = lets  # bindings needed before a store: [(name, expr)]
        self.place = place  # the place text, using those bindings


def make_set(lets, store):
    """A setter: `store(v)` stores into the place.  The value and the indices are evaluated first,
    because an IndexMut receiver borrows the array during the evaluation of later operands."""

    def setter(v):
        stmts = []
        vv = v
        if lets and "self." in v:
            stmts.append("let v_ = %s;" % v)
            vv = "v_"
        elif "self." in v and store is not None and store("X").find("[") >= 0:
            stmts.append("let v_ = %s;" % v)
            vv = "v_"
        for n, ex in lets:
            stmts.append("let %s = %s;" % (n, ex))
        stmts.append(store(vv))
        if len(stmts) == 1:
            return stmts[0]
        return "{ " + " ".join(x if x.endswith(";") else x + ";" for x in stmts) + " }"

    return setter


def unify(a, b):
    """Numeric promotion, as Pascal does implicitly."""
    if a.ty == b.ty:
        return a.code, b.code, a.ty
    if "f64" in (a.ty, b.ty):
        return (a.code if a.ty == "f64" else "(%s as f64)" % a.code,
                b.code if b.ty == "f64" else "(%s as f64)" % b.code, "f64")
    if "i64" in (a.ty, b.ty):
        return (a.code if a.ty == "i64" else "(%s as i64)" % a.code,
                b.code if b.ty == "i64" else "(%s as i64)" % b.code, "i64")
    return a.code, b.code, a.ty


WORD_FIELDS = {
    "int": ("int", "set_int", "i32"),
    "sc": ("sc", "set_sc", "i32"),
    "gr": ("gr", "set_gr", "f64"),
}
WORD_HH = {"rh": "rh", "lh": "lh", "b0": "b0", "b1": "b1"}
WORD_Q = {"b0": "q0", "b1": "q1", "b2": "q2", "b3": "q3"}

# Literal-string call conventions.  (callee, index of the argument) -> byte-slice.
STR_VARIANT = {"print": "print_s", "print_nl": "print_nl_s", "print_esc": "print_esc_s",
               "pdf_print": "pdf_print_s", "pdf_print_ln": "pdf_print_ln_s", "print_err": "print_err_s"}


class Translator:
    def __init__(self, gen):
        self.g = gen  # the Gen object: globals, consts, macro lookups
        self.param_ty = "i32"
        self.current = None
        self.memo = {}
        self.busy = set()
        self.fail = {}

    # -- entry -----------------------------------------------------------
    def method(self, name):
        """Returns dict(params, ret, get, set, mut) or raises NoTrans."""
        if name in self.memo:
            return self.memo[name]
        if name in self.fail:
            raise NoTrans(self.fail[name])
        if name in self.busy:
            raise NoTrans("recursive " + name)
        self.busy.add(name)
        saved = self.current
        self.current = name
        try:
            m = self.g.macro(name)
            if m is None:
                raise NoTrans("no macro " + name)
            r = self._translate(m)
            self.memo[name] = r
            return r
        except NoTrans as e:
            self.fail[name] = str(e)
            raise
        finally:
            self.busy.discard(name)
            self.current = saved

    def _translate(self, m):
        toks = tokenize(m.body)
        if not toks:
            raise NoTrans("empty")
        # A non-parametric alias of a parametric macro or procedure: font==type
        if not m.param and len(toks) == 1 and toks[0][0] == "id":
            target = toks[0][1]
            tm = self.g.macro(target)
            if tm is not None and (tm.param or self._is_alias(target)):
                t = self.method(target)
                if t["params"]:
                    return dict(alias=target, params=t["params"], ret=t["ret"], mut=t["mut"],
                                get=None, set=t.get("set") is not None, stmt=t.get("stmt", False))
        # param_ty is per-parse state; a recursive translation saves and restores it.
        param_ty = self._param_type(m, toks) if m.param else None
        p = Parser(self, toks, m.param, None, param_ty if param_ty else "i32")
        if self._is_stmt(toks):
            code = p.parse_stmts()
            if not p.eof():
                raise NoTrans("trailing tokens")
            return dict(params=[param_ty] if m.param else [], ret=None, get=code, set=False,
                        mut=True, stmt=True)
        e = p.parse_expr()
        if not p.eof():
            raise NoTrans("trailing tokens")
        if e.ty == "void":
            return dict(params=[param_ty] if m.param else [], ret=None, get=e.code + ";", set=False,
                        mut=True, stmt=True)
        setter = e.set if (param_ty in (None, "i32", "i64")) else None
        return dict(params=[param_ty] if m.param else [], ret=e.ty, get=e.code, set=setter,
                    mut=e.mut, stmt=False, place=e.place if not m.param else None, lets=e.lets)

    def _is_alias(self, name):
        m = self.g.macro(name)
        if m is None or m.param:
            return False
        t = tokenize(m.body)
        if len(t) != 1 or t[0][0] != "id":
            return False
        tm = self.g.macro(t[0][1])
        return tm is not None and (tm.param or self._is_alias(t[0][1]))

    @staticmethod
    def _is_stmt(toks):
        for k, v in toks:
            if k == "op" and v in (":=", ";"):
                return True
            if k == "id" and v in ("begin", "if", "while", "for", "incr", "decr", "repeat", "stat", "tats"):
                return True
        return False

    def _param_type(self, m, toks):
        for i, (k, v) in enumerate(toks):
            if k == "op" and v == "#":
                nx = toks[i + 1] if i + 1 < len(toks) else None
                if nx == ("op", "."):
                    fld = toks[i + 2][1]
                    if fld in ("hh", "int", "sc", "gr", "qqqq"):
                        return "Word"
                    if fld in ("b0", "b1", "b2", "b3"):
                        return "FourQuarters"
                pv = toks[i - 1] if i > 0 else None
                ppv = toks[i - 2] if i > 1 else None
                if pv == ("op", "(") and ppv and ppv[0] == "id":
                    if self.g.is_proc(ppv[1]) and self.g.proc_info(ppv[1]) is not None:
                        t = self.g.param_rust_type(ppv[1], 0)
                        if t in ("f64", "Word", "FourQuarters"):
                            return t
                    tm = self.g.macro(ppv[1])
                    if tm is not None and tm.param:
                        try:
                            t = self.method(ppv[1])
                            if t["params"] and t["params"][0] != "i32":
                                return t["params"][0]
                        except NoTrans:
                            pass
        return "i32"


class Parser:
    def __init__(self, tr, toks, has_param, param_expr=None, param_ty=None):
        self.tr = tr
        self.g = tr.g
        self.t = toks
        self.i = 0
        self.has_param = has_param
        self.param_expr = param_expr  # inline expansion: `#` stands for this expression
        self.param_ty = param_ty if param_ty is not None else tr.param_ty

    def eof(self):
        return self.i >= len(self.t)

    def peek(self, k=0):
        return self.t[self.i + k] if self.i + k < len(self.t) else (None, None)

    def eat(self, v):
        if self.peek() == ("op", v) or self.peek() == ("id", v):
            self.i += 1
            return True
        return False

    def need(self, v):
        if not self.eat(v):
            raise NoTrans("expected %s got %r" % (v, self.peek()))

    # -- expressions -----------------------------------------------------
    def parse_expr(self):
        a = self.parse_simple()
        k, v = self.peek()
        if k == "op" and v in ("=", "<>", "<", ">", "<=", ">="):
            self.i += 1
            b = self.parse_simple()
            op = {"=": "==", "<>": "!="}.get(v, v)
            x, y, _ = unify(a, b)
            return E("(%s %s %s)" % (x, op, y), "bool", None, a.mut or b.mut)
        return a

    def parse_simple(self):
        neg = False
        if self.eat("-"):
            neg = True
        elif self.eat("+"):
            pass
        a = self.parse_term()
        if neg:
            a = E("-%s" % a.code, a.ty, None, a.mut)
        while True:
            k, v = self.peek()
            if k == "op" and v in ("+", "-"):
                self.i += 1
                b = self.parse_term()
                x, y, ty = unify(a, b)
                a = E("%s %s %s" % (x, v, y), ty, None, a.mut or b.mut)
            elif k == "id" and v == "or":
                self.i += 1
                b = self.parse_term()
                a = E("(%s || %s)" % (a.code, b.code), "bool", None, a.mut or b.mut)
            else:
                return a

    def parse_term(self):
        a = self.parse_factor()
        while True:
            k, v = self.peek()
            if k == "op" and v == "*":
                self.i += 1
                b = self.parse_factor()
                a = E("%s * %s" % (self.par(a), self.par(b)), a.ty, None, a.mut or b.mut)
            elif k == "id" and v == "div":
                self.i += 1
                b = self.parse_factor()
                a = E("%s / %s" % (self.par(a), self.par(b)), a.ty, None, a.mut or b.mut)
            elif k == "id" and v == "mod":
                self.i += 1
                b = self.parse_factor()
                a = E("%s %% %s" % (self.par(a), self.par(b)), a.ty, None, a.mut or b.mut)
            elif k == "id" and v == "and":
                self.i += 1
                b = self.parse_factor()
                if a.ty == "i32" and b.ty == "i32":  # only pdf_action_named_id(p) and 1, §1536
                    a = E("(%s & %s)" % (a.code, b.code), "i32", None, a.mut or b.mut)
                else:
                    a = E("(%s && %s)" % (a.code, b.code), "bool", None, a.mut or b.mut)
            else:
                return a

    @staticmethod
    def par(e):
        if re.fullmatch(r"[A-Za-z_0-9.()\[\]]+|\(.*\)", e.code) and "+" not in e.code and "-" not in e.code:
            return e.code
        if e.code.startswith("(") and e.code.endswith(")") and e.code.count("(") == 1:
            return e.code
        return "(%s)" % e.code

    def parse_factor(self):
        k, v = self.peek()
        if k == "num":
            self.i += 1
            return E(str(v))
        if k == "real":
            self.i += 1
            return E(v, "f64")
        if k == "char":
            self.i += 1
            return E(char_lit(v))
        if k == "str":
            self.i += 1
            return E(rust_bytes(v), "bytes")
        if k == "op" and v == "(":
            self.i += 1
            e = self.parse_expr()
            self.need(")")
            return self.selectors(E("(%s)" % e.code, e.ty, e.set, e.mut, e.lets, e.place))
        if k == "op" and v == "#":
            self.i += 1
            if not self.has_param:
                raise NoTrans("# in parameterless macro")
            if self.param_expr is not None:
                return self.selectors(self.param_expr)
            return self.selectors(E("p", self.param_ty))
        if k == "id" and v == "not":
            self.i += 1
            e = self.parse_factor()
            return E("!%s" % e.code, "bool", None, e.mut)
        if k == "id":
            self.i += 1
            return self.ident(v)
        raise NoTrans("factor %r" % (self.peek(),))

    def args(self):
        self.need("(")
        out = []
        while True:
            out.append(self.parse_expr())
            if self.eat(","):
                continue
            self.need(")")
            return out

    def ident(self, n):
        g = self.g
        if n in ("true", "false"):
            return E(n, "bool")
        if n == "odd":
            a = self.args()
            return E("((%s & 1) != 0)" % a[0].code, "bool", None, a[0].mut)
        if n in ("qi", "qo", "hi", "ho", "si", "so", "float", "unfloat"):
            a = self.args()
            return self.selectors(E(a[0].code if re.fullmatch(r"[\w.()\[\]]+", a[0].code) else "(%s)" % a[0].code, a[0].ty, a[0].set, a[0].mut, a[0].lets, a[0].place))
        if n == "abs":
            a = self.args()
            return E("abs(%s)" % a[0].code, "i32", None, a[0].mut)
        c = g.const_ref(n)
        if c is not None:
            return E(c)
        m = g.macro(n)
        if m is not None:
            return self.macro_ref(n, m)
        if g.is_global(n):
            return self.global_ref(n)
        if g.is_proc(n):
            return self.proc_ref(n)
        raise NoTrans("unknown %s" % n)

    def macro_ref(self, n, m):
        t = self.tr.method(n)
        rn = self.g.rust_name(n)
        if t["params"]:
            if self.peek() != ("op", "("):
                raise NoTrans("macro %s used without arguments" % n)
            a = self.args()
            if len(a) != 1:
                raise NoTrans("macro %s with %d arguments" % (n, len(a)))
            arg = a[0]
            if t["params"][0] in ("Word", "FourQuarters"):
                # A macro over a word, like equiv_field(eqtb[p]): expand it in place, so it stays
                # an lvalue.
                sub = Parser(self.tr, tokenize(m.body), True, arg, t["params"][0])
                r = sub.parse_expr()
                if not sub.eof():
                    raise NoTrans("trailing tokens in inline expansion")
                return self.selectors(r)
            setter = None
            if t.get("set"):
                def setter(v, rn=rn, code=arg.code):
                    return "self.set_%s(%s, %s)" % (rn, code, v)
            return self.selectors(E("self.%s(%s)" % (rn, arg.code), t["ret"] or "void", setter, t["mut"] or arg.mut))
        setter = None
        if t.get("set"):
            def setter(v, rn=rn):
                return "self.set_%s(%s)" % (rn, v)
        if t.get("place") and (t["ret"] == "Word" or t["ret"] in self.g.record_fields):
            # aux.sc must store into cur_list.aux_field, not into a copy returned by self.aux().
            return self.selectors(E(t["place"], t["ret"], setter, t["mut"], t.get("lets", ()), t["place"]))
        return self.selectors(E("self.%s()" % rn, t["ret"] or "void", setter, t["mut"]))

    def global_ref(self, n):
        g = self.g
        if n in SHORT_GLOBALS and self.tr.current not in SHORT_GLOBAL_OK:
            raise NoTrans("%s may be a local at the use site" % n)
        info = g.global_info(n)
        rn = g.rust_name(info.name)
        if info.scalar:
            place = "self.%s" % rn
            e = E(place, info.ty, make_set((), lambda v, pl=place: "%s = %s" % (pl, v)), False, (), place)
            return self.selectors(e)
        if self.peek() != ("op", "["):
            raise NoTrans("array %s without subscript" % n)
        self.i += 1
        ix = self.parse_expr()
        self.need("]")
        compact = info.elem in ("u8", "u16")
        # An index that calls a method must be evaluated before the array is borrowed.
        lets = ()
        ic = ix.code
        if "self." in ic and "(" in ic:
            lets = (("i_", ic),)
            ic = "i_"
        if compact:
            e = E("self.%s.at(%s)" % (rn, ix.code), "i32",
                  make_set(lets, lambda v, rn=rn, c=ic: "self.%s.put(%s, %s)" % (rn, c, v)), ix.mut)
            return e
        ty = info.elem
        code = "self.%s[%s]" % (rn, ix.code)
        place = "self.%s[%s]" % (rn, ic)
        e = E(code, ty, make_set(lets, lambda v, pl=place: "%s = %s" % (pl, v)), ix.mut, lets, place)
        return self.selectors(e)

    def proc_ref(self, n):
        g = self.g
        p = g.proc_info(n)
        rn = g.rust_name(n)
        args = []
        if self.peek() == ("op", "("):
            self.i += 1
            ai = 0
            while True:
                k, v = self.peek()
                if k == "str" and (self.peek(1) == ("op", ",") or self.peek(1) == ("op", ")")):
                    self.i += 1
                    if n in STR_VARIANT:
                        rn = STR_VARIANT[n]
                    args.append(rust_bytes(v))
                else:
                    e = self.parse_expr()
                    if e.ty == "void":
                        raise NoTrans("void arg")
                    code = e.code
                    if p is not None and ai < len(p.params):
                        from_ty = g.param_rust_type(n, ai)
                        if from_ty == "f64" and e.ty == "i32":
                            code = "%s as f64" % code
                    args.append(code)
                ai += 1
                if self.eat(","):
                    continue
                self.need(")")
                break
        if p is not None and len(args) != len(p.params):
            raise NoTrans("%s takes %d arguments, got %d" % (n, len(p.params), len(args)))
        return E("self.%s(%s)" % (rn, ", ".join(args)), g.proc_ret(n) or "void", None, not g.is_pure(n))

    def selectors(self, e):
        while self.peek() == ("op", "."):
            fld = self.peek(1)[1]
            if e.ty == "Word":
                self.i += 2
                # A word without a place is a value, like a parameter: no setter.
                settable = e.place is not None
                pl = e.place if settable else e.code
                lets = e.lets
                if fld in WORD_HH:  # a two_halves variable: prim[p].rh
                    e = E("%s.%s()" % (e.code, fld), "i32",
                          make_set(lets, lambda v, pl=pl, st=fld: "%s.set_%s(%s)" % (pl, st, v)) if settable else None, e.mut)
                elif fld == "hh":
                    self.need(".")
                    f2 = self.peek()[1]
                    self.i += 1
                    if f2 not in WORD_HH:
                        raise NoTrans("hh." + str(f2))
                    e = E("%s.%s()" % (e.code, f2), "i32",
                          make_set(lets, lambda v, pl=pl, st=f2: "%s.set_%s(%s)" % (pl, st, v)) if settable else None, e.mut)
                elif fld in WORD_FIELDS:
                    g, st, ty = WORD_FIELDS[fld]
                    e = E("%s.%s()" % (e.code, g), ty,
                          make_set(lets, lambda v, pl=pl, st=st: "%s.%s(%s)" % (pl, st, v)) if settable else None, e.mut)
                elif fld == "qqqq":
                    if self.peek() == ("op", ".") and self.peek(1)[1] in WORD_Q:
                        self.i += 1
                        f2 = self.peek()[1]
                        self.i += 1
                        q = WORD_Q[f2]
                        e = E("%s.%s()" % (e.code, q), "i32",
                              make_set(lets, lambda v, pl=pl, q=q: "%s.set_%s(%s)" % (pl, q, v)) if settable else None, e.mut)
                    else:
                        e = E("%s.qqqq()" % e.code, "FourQuarters",
                              make_set(lets, lambda v, pl=pl: "%s.set_qqqq(%s)" % (pl, v)) if settable else None, e.mut)
                else:
                    raise NoTrans("word field " + str(fld))
            elif e.ty in self.g.record_fields:
                self.i += 2
                fields = self.g.record_fields[e.ty]
                if fld not in fields:
                    raise NoTrans("record field " + str(fld))
                code = "%s.%s" % (e.code, fld)
                if e.place is not None:
                    pl = "%s.%s" % (e.place, fld)
                    e = E(code, fields[fld], make_set(e.lets, lambda v, pl=pl: "%s = %s" % (pl, v)), e.mut, e.lets, pl)
                else:
                    e = E(code, fields[fld], None, e.mut)
            else:
                raise NoTrans("select from %s" % e.ty)
        return e

    # -- statements ------------------------------------------------------
    def parse_stmts(self):
        out = []
        while not self.eof():
            if self.eat(";"):
                continue
            if self.peek() in (("id", "end"), ("id", "else"), ("id", "until")):
                break
            out.append(self.parse_stmt())
        return "\n".join(s for s in out if s)

    def parse_stmt(self):
        k, v = self.peek()
        if k == "id":
            if v == "begin":
                self.i += 1
                body = self.parse_stmts()
                self.need("end")
                return body
            if v in ("stat", "tats"):
                self.i += 1
                return ""
            if v == "debug":
                while not self.eof() and self.peek() != ("id", "gubed"):
                    self.i += 1
                self.need("gubed")
                return ""
            if v == "if":
                self.i += 1
                c = self.parse_expr()
                self.need("then")
                a = self.parse_stmt()
                b = None
                if self.eat("else"):
                    b = self.parse_stmt()
                s = "if %s { %s }" % (self.strip(c.code), a + ("" if a.endswith(";") or not a else ";"))
                if b is not None:
                    s += " else { %s }" % (b + ("" if b.endswith(";") or not b else ";"))
                return s
            if v == "while":
                self.i += 1
                c = self.parse_expr()
                self.need("do")
                a = self.parse_stmt()
                return "while %s { %s }" % (self.strip(c.code), a + ("" if a.endswith(";") or not a else ";"))
            if v in ("incr", "decr"):
                self.i += 1
                self.need("(")
                e = self.parse_expr()
                self.need(")")
                if e.set is None:
                    raise NoTrans("incr of non-lvalue")
                return e.set("%s %s 1" % (e.code, "+" if v == "incr" else "-")) + ";"
            if v in ("goto", "for", "repeat", "case", "return"):
                raise NoTrans(v)
        e = self.parse_expr()
        if self.eat(":="):
            r = self.parse_expr()
            if e.set is None:
                raise NoTrans("assign to non-lvalue")
            return e.set(r.code) + ";"
        return e.code + ";"

    @staticmethod
    def strip(c):
        if c.startswith("(") and c.endswith(")"):
            d = 0
            for i, ch in enumerate(c):
                if ch == "(":
                    d += 1
                elif ch == ")":
                    d -= 1
                    if d == 0 and i != len(c) - 1:
                        return c
            return c[1:-1]
        return c

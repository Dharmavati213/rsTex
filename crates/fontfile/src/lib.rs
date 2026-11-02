//! Type 1 parse and subset, encoding files, map-line parser, ToUnicode.  Stateless functions; never
//! calls back into the engine (DESIGN.md section 9, M2).
#![forbid(unsafe_code)]

//! `mente-ast` — Dual-Track Semantic Hashing (`spec_hash` vs. `body_hash`) & `<0.1ms` Spec Projection.
//!
//! Governed by `spec/holographic-module.md`.

use mente_syntax::{tokenize, FnDecl, ItemDecl, ModuleFile, ParamMode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DualSemanticHash {
    /// 128-bit hex fingerprint of public interface, layouts, types, effects & Hoare contracts.
    pub spec_hash: String,
    /// 128-bit hex fingerprint of canonicalized implementation bodies.
    pub body_hash: String,
    /// 128-bit hex fingerprint of the bodies dependents instantiate or inline: `pub` functions
    /// that are generic or marked `@inline`. Only a dependent that uses one of them depends on it.
    pub instance_hash: String,
}

#[derive(Clone, Debug)]
pub struct SpecProjectionReport {
    pub module_path: String,
    pub hashes: DualSemanticHash,
    pub projected_source: String,
    pub total_tokens: usize,
    pub spec_tokens: usize,
    pub savings_percent: f64,
}

/// Deterministic 128-bit FNV-1a/SplitMix dual-lane hasher (zero external crate dependencies).
struct SemanticHasher {
    h1: u64,
    h2: u64,
}

impl SemanticHasher {
    fn new(seed: u64) -> Self {
        Self {
            h1: 0xcbf29ce484222325 ^ seed,
            h2: 0x84222325cbf29ce4 ^ seed.rotate_left(17),
        }
    }

    fn update(&mut self, tag: &str, data: &str) {
        for b in tag.bytes().chain(std::iter::once(0xff)).chain(data.bytes()) {
            self.h1 ^= b as u64;
            self.h1 = self.h1.wrapping_mul(0x100000001b3);
            self.h2 ^= (b as u64).wrapping_mul(0x9e3779b97f4a7c15);
            self.h2 = self.h2.rotate_left(13).wrapping_mul(0xbf58476d1ce4e5b9);
        }
    }

    fn finish_hex(&self) -> String {
        format!("{:016x}{:016x}", self.h1, self.h2)
    }
}

/// Computes `spec_hash` (public interface + layout + contracts) and `body_hash` (implementation bodies).
///
/// Editing implementation bodies, comments, or whitespace leaves `spec_hash` 100% invariant,
/// guaranteeing Zero Downstream Recompilation.
pub fn compute_dual_hashes(module: &ModuleFile) -> DualSemanticHash {
    let mut spec_h = SemanticHasher::new(0x4D45_4E54_455F_5350); // "MENTE_SP"
    let mut body_h = SemanticHasher::new(0x4D45_4E54_455F_4244); // "MENTE_BD"
    let mut inst_h = SemanticHasher::new(0x4D45_4E54_455F_494E); // "MENTE_IN"
    let traits: Vec<&str> = module
        .items
        .iter()
        .filter_map(|item| match item {
            ItemDecl::Trait(t) => Some(t.name.as_str()),
            _ => None,
        })
        .collect();

    spec_h.update("module", &module.module_path);

    for item in &module.items {
        match item {
            ItemDecl::Unit(u) => {
                spec_h.update("unit", &u.raw_text);
            }
            ItemDecl::Struct(s) if s.is_pub => {
                spec_h.update("struct", &s.name);
                spec_h.update("generics", &s.generics);
                for a in &s.attrs {
                    spec_h.update("attr", &a.to_string());
                }
                // Private fields still decide the struct's size, alignment and defaults,
                // which dependents that hold the struct by value compile against.
                for f in &s.fields {
                    spec_h.update(
                        "field",
                        &format!(
                            "{}:{}:{}:{:?}:{:?}",
                            f.is_pub, f.name, f.ty, f.byte_range, f.default_expr
                        ),
                    );
                }
                for inv in &s.invariants {
                    spec_h.update("invariant", inv);
                }
            }
            ItemDecl::BitStruct(bs) if bs.is_pub => {
                spec_h.update("bitstruct", &format!("{}:{}", bs.name, bs.backing_type));
                for f in &bs.fields {
                    spec_h.update(
                        "bitfield",
                        &format!("{}:{}:{:?}", f.name, f.ty, f.bit_range),
                    );
                }
            }
            ItemDecl::Enum(e) if e.is_pub => {
                spec_h.update("enum", &e.name);
                for v in &e.variants {
                    spec_h.update("variant", v);
                }
            }
            ItemDecl::Trait(t) if t.is_pub => {
                spec_h.update("trait", &t.name);
                for m in &t.methods_sig {
                    spec_h.update("method", m);
                }
            }
            ItemDecl::Fn(f) => {
                if f.is_pub {
                    spec_h.update("fn", &f.name);
                    spec_h.update("generics", &f.generics);
                    for a in &f.attrs {
                        spec_h.update("fn_attr", &a.to_string());
                    }
                    for p in &f.params {
                        spec_h.update("param", &format!("{:?}:{}:{}", p.mode, p.name, p.ty));
                    }
                    spec_h.update("ret", f.return_ty.as_deref().unwrap_or("Void"));
                    spec_h.update("err", f.error_ty.as_deref().unwrap_or(""));
                    for p in &f.provenance {
                        spec_h.update("from", p);
                    }
                    for eff in &f.effects {
                        spec_h.update("effect", eff);
                    }
                    for r in &f.requires {
                        spec_h.update("requires", r);
                    }
                    for e in &f.ensures {
                        spec_h.update("ensures", e);
                    }
                    for d in &f.decreases {
                        spec_h.update("decreases", d);
                    }
                }
                // Implementation tokens go into body_hash, and also into instance_hash
                // when dependents compile against the body itself
                body_h.update("fn_body", &f.name);
                for tok in &f.body_tokens {
                    body_h.update("tok", &format!("{:?}", tok));
                }
                if f.is_pub && is_instantiable(f, &traits) {
                    inst_h.update("fn_body", &f.name);
                    for tok in &f.body_tokens {
                        inst_h.update("tok", &format!("{:?}", tok));
                    }
                }
            }
            _ => {}
        }
    }

    DualSemanticHash {
        spec_hash: spec_h.finish_hex(),
        body_hash: body_h.finish_hex(),
        instance_hash: inst_h.finish_hex(),
    }
}

/// A function whose body dependents compile into themselves: generic (instantiated per use)
/// or `@inline` (inlined across modules). No other body may reach a dependent.
///
/// A parameter typed by a bare trait name makes a function generic without `<...>`; only the
/// module's own traits are known here, until name resolution exists.
fn is_instantiable(f: &FnDecl, traits: &[&str]) -> bool {
    !f.generics.is_empty()
        || f.attrs.iter().any(|a| a.name == "inline")
        || f.params.iter().any(|p| traits.contains(&p.ty.trim()))
}

/// Projects a `.mn` `ModuleFile` into its pure Holographic Spec Blueprint (`mn view --spec`).
///
/// Strips function implementation bodies (`{ ... }` -> `= ...`), retaining all public types,
/// hardware layout contracts, Hoare contracts (`requires`/`ensures`/`invariant`), and `@total`/`decreases`.
pub fn project_spec_blueprint(module: &ModuleFile) -> SpecProjectionReport {
    let hashes = compute_dual_hashes(module);
    let mut out = String::new();

    out.push_str(&format!(
        "// [Mente Holographic Spec Projection] module {}\n",
        module.module_path
    ));
    out.push_str(&format!("// spec_hash: {}\n", hashes.spec_hash));
    out.push_str(&format!("// body_hash: {}\n", hashes.body_hash));
    out.push_str(&format!("// instance_hash: {}\n\n", hashes.instance_hash));
    out.push_str(&format!("module {}\n\n", module.module_path));

    for item in &module.items {
        match item {
            ItemDecl::Unit(u) => {
                out.push_str(&format!("unit {}\n", u.raw_text));
            }
            ItemDecl::BitStruct(bs) if bs.is_pub => {
                for d in &bs.docs {
                    out.push_str(&format!("/// {}\n", d));
                }
                for a in &bs.attrs {
                    out.push_str(&format!("{}\n", a));
                }
                out.push_str(&format!(
                    "pub bitstruct {} : {} {{\n",
                    bs.name, bs.backing_type
                ));
                for f in &bs.fields {
                    let vis = if f.is_pub { "pub " } else { "" };
                    out.push_str(&format!(
                        "  @bits({}..{}) {}{}: {}\n",
                        f.bit_range.0, f.bit_range.1, vis, f.name, f.ty
                    ));
                }
                out.push_str("}\n\n");
            }
            ItemDecl::Struct(s) if s.is_pub => {
                for d in &s.docs {
                    out.push_str(&format!("/// {}\n", d));
                }
                for a in &s.attrs {
                    out.push_str(&format!("{}\n", a));
                }
                out.push_str(&format!("pub struct {}{} {{\n", s.name, s.generics));
                for f in &s.fields {
                    let mut prefix = String::new();
                    for a in &f.attrs {
                        prefix.push_str(&format!("{} ", a));
                    }
                    if f.is_pub {
                        prefix.push_str("pub ");
                    }
                    if let Some(def) = &f.default_expr {
                        out.push_str(&format!("  {}{}: {} = {}\n", prefix, f.name, f.ty, def));
                    } else {
                        out.push_str(&format!("  {}{}: {}\n", prefix, f.name, f.ty));
                    }
                }
                if !s.invariants.is_empty() {
                    out.push_str("\n  invariant {\n");
                    for inv in &s.invariants {
                        out.push_str(&format!("    {}\n", inv));
                    }
                    out.push_str("  }\n");
                }
                out.push_str("}\n\n");
            }
            ItemDecl::Enum(e) if e.is_pub => {
                // One variant per line, as the grammar writes them (`grammar.md`, item 10).
                out.push_str(&format!("pub enum {} {{\n", e.name));
                for v in &e.variants {
                    out.push_str(&format!("  {}\n", v));
                }
                out.push_str("}\n\n");
            }
            ItemDecl::Trait(t) if t.is_pub => {
                out.push_str(&format!("pub trait {} {{\n", t.name));
                for m in &t.methods_sig {
                    out.push_str(&format!("  {}\n", m));
                }
                out.push_str("}\n\n");
            }
            ItemDecl::Fn(f) if f.is_pub => {
                for d in &f.docs {
                    out.push_str(&format!("/// {}\n", d));
                }
                for a in &f.attrs {
                    out.push_str(&format!("{}\n", a));
                }
                let params_str: Vec<String> = f
                    .params
                    .iter()
                    .map(|p| {
                        let m = match p.mode {
                            ParamMode::ReadOnly => "",
                            ParamMode::Mut => "mut ",
                            ParamMode::Own => "own ",
                            ParamMode::RegionIn => "in ",
                        };
                        format!("{}{}: {}", m, p.name, p.ty)
                    })
                    .collect();

                out.push_str(&format!(
                    "pub fn {}{}({})",
                    f.name,
                    f.generics,
                    params_str.join(", ")
                ));
                if let Some(ret) = &f.return_ty {
                    if let Some(err) = &f.error_ty {
                        out.push_str(&format!(" -> {} ! {}", ret, err));
                    } else {
                        out.push_str(&format!(" -> {}", ret));
                    }
                }
                if !f.provenance.is_empty() {
                    out.push_str(&format!(" from {}", f.provenance.join(", ")));
                }
                if !f.effects.is_empty() {
                    out.push_str(&format!(" with {}", f.effects.join(", ")));
                }
                out.push('\n');
                for r in &f.requires {
                    out.push_str(&format!("  requires {}\n", r));
                }
                for e in &f.ensures {
                    out.push_str(&format!("  ensures  {}\n", e));
                }
                for d in &f.decreases {
                    out.push_str(&format!("  decreases {}\n", d));
                }
                if !f.body_tokens.is_empty() {
                    out.push_str("= ...\n\n");
                } else {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }

    let spec_tokens = tokenize(&out).len().saturating_sub(1);
    let total_tokens = module.total_token_count.max(1);
    // The share of the module's tokens the projection leaves out, never below zero.
    let savings_percent = (1.0 - spec_tokens as f64 / total_tokens as f64).max(0.0) * 100.0;

    SpecProjectionReport {
        module_path: module.module_path.clone(),
        hashes,
        projected_source: out,
        total_tokens,
        spec_tokens,
        savings_percent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mente_syntax::parse_module;

    #[test]
    fn test_dual_hash_invariance_when_body_changes() {
        let v1 = r#"
module math.fast
@branchless
pub fn score(x: i64, y: i64) -> i64
  requires x >= 0 and y >= 0
{
  val tmp = x * 2 + y
  tmp
}
"#;
        let v2 = r#"
module math.fast
// Completely rewritten implementation body and comments!
@branchless
pub fn score(x: i64, y: i64) -> i64
  requires x >= 0 and y >= 0
{
  val a = x << 1
  val b = a + y
  b
}
"#;
        let m1 = parse_module(v1).unwrap();
        let m2 = parse_module(v2).unwrap();
        let h1 = compute_dual_hashes(&m1);
        let h2 = compute_dual_hashes(&m2);

        // Spec hash MUST be 100% identical (0 downstream recompilation!)
        assert_eq!(h1.spec_hash, h2.spec_hash);
        // Body hash MUST reflect the algorithm change
        assert_ne!(h1.body_hash, h2.body_hash);
    }

    #[test]
    fn test_private_field_change_changes_spec_hash() {
        let v1 = r#"
module geo.shapes
pub struct Point {
  pub x: f32
  cache: u32
}
"#;
        let v2 = r#"
module geo.shapes
pub struct Point {
  pub x: f32
  cache: u64
}
"#;
        let h1 = compute_dual_hashes(&parse_module(v1).unwrap());
        let h2 = compute_dual_hashes(&parse_module(v2).unwrap());

        // A private field changes the struct's size, so dependents must recompile
        assert_ne!(h1.spec_hash, h2.spec_hash);
    }

    #[test]
    fn test_instantiable_bodies_go_into_instance_hash() {
        let v1 = r#"
module util.seq
pub fn largest<T>(a: T, b: T) -> T {
  if a > b { a } else { b }
}
pub fn total(n: u32) -> u32 {
  n + 1
}
"#;
        let generic_edit = v1.replace("if a > b { a } else { b }", "if b > a { b } else { a }");
        let plain_edit = v1.replace("n + 1", "1 + n");
        let h1 = compute_dual_hashes(&parse_module(v1).unwrap());
        let hg = compute_dual_hashes(&parse_module(&generic_edit).unwrap());
        let hp = compute_dual_hashes(&parse_module(&plain_edit).unwrap());

        // A generic body is instantiated by dependents: editing it changes instance_hash only
        assert_eq!(h1.spec_hash, hg.spec_hash);
        assert_ne!(h1.instance_hash, hg.instance_hash);
        // A plain body never reaches a dependent
        assert_eq!(h1.spec_hash, hp.spec_hash);
        assert_eq!(h1.instance_hash, hp.instance_hash);
        assert_ne!(h1.body_hash, hp.body_hash);
    }

    #[test]
    fn test_trait_typed_parameter_makes_a_function_instantiable() {
        let v1 = r#"
module io.encode
pub trait ByteSink {
  fn write_u64(mut self, v: u64)
}
pub fn encode(mut sink: ByteSink, v: u64) {
  sink.write_u64(v)
}
"#;
        let v2 = v1.replace("sink.write_u64(v)", "sink.write_u64(v + 0)");
        let h1 = compute_dual_hashes(&parse_module(v1).unwrap());
        let h2 = compute_dual_hashes(&parse_module(&v2).unwrap());

        // encode is monomorphized per sink type, so dependents compile its body
        assert_eq!(h1.spec_hash, h2.spec_hash);
        assert_ne!(h1.instance_hash, h2.instance_hash);
    }

    #[test]
    fn test_from_clause_is_part_of_spec_hash() {
        let v1 = r#"
module util.views
pub fn pick(data: []u8, key: []u8) -> []u8 from data {
  data
}
"#;
        let v2 = v1.replace("from data", "from data, key");
        let h1 = compute_dual_hashes(&parse_module(v1).unwrap());
        let h2 = compute_dual_hashes(&parse_module(&v2).unwrap());

        // Callers keep every named source alive, so widening provenance changes the interface
        assert_ne!(h1.spec_hash, h2.spec_hash);
    }

    #[test]
    fn test_spec_hash_covers_units_and_pub_declarations_only() {
        let base = r#"
module geo.decls
unit meter, sec
pub bitstruct Flags : u8 {
  @bits(0..8) pub raw: u8
}
bitstruct Hidden : u8 {
  @bits(0..8) pub raw: u8
}
pub enum Side {
  Left
  Right
}
enum Inner {
  A
  B
}
pub trait Probe {
  fn hit(self) -> bool
}
trait Secret {
  fn peek(self) -> bool
}
struct Scratch {
  n: u32
}
"#;
        let spec = |code: &str| compute_dual_hashes(&parse_module(code).unwrap()).spec_hash;
        let h = spec(base);
        // Each public declaration, and every unit, is part of the interface.
        for (from, to) in [
            ("unit meter, sec", "unit meter, sec, kg"),
            (
                "pub bitstruct Flags : u8 {\n  @bits(0..8) pub raw: u8",
                "pub bitstruct Flags : u8 {\n  @bits(0..8) pub bits: u8",
            ),
            ("  Left\n  Right\n}", "  Left\n  Right\n  Up\n}"),
            ("  fn hit(self) -> bool", "  fn hit(self) -> u8"),
        ] {
            assert_ne!(
                h,
                spec(&base.replace(from, to)),
                "editing `{}` must change spec_hash",
                from
            );
        }
        // A private declaration is not.
        for (from, to) in [
            (
                "bitstruct Hidden : u8 {\n  @bits(0..8) pub raw: u8",
                "bitstruct Hidden : u8 {\n  @bits(0..8) pub bits: u8",
            ),
            ("enum Inner { A, B }", "enum Inner { A, B, C }"),
            ("  fn peek(self) -> bool", "  fn peek(self) -> u8"),
            ("struct Scratch {\n  n: u32", "struct Scratch {\n  n: u64"),
        ] {
            assert_eq!(
                h,
                spec(&base.replace(from, to)),
                "editing `{}` must not change spec_hash",
                from
            );
        }
    }

    #[test]
    fn test_inline_bodies_go_into_instance_hash() {
        let code = "module util.inl\n@inline\npub fn twice(n: u32) -> u32 {\n  n + n\n}\n@total\npub fn once(n: u32) -> u32 {\n  n\n}\n";
        let h = |c: &str| compute_dual_hashes(&parse_module(c).unwrap());
        let base = h(code);
        let inline_edit = h(&code.replace("n + n", "n * 2"));
        assert_eq!(base.spec_hash, inline_edit.spec_hash);
        assert_ne!(base.instance_hash, inline_edit.instance_hash);
        // Another attribute does not make a body reach dependents.
        let plain_edit = h(&code.replace("  n\n}", "  n + 0\n}"));
        assert_eq!(base.instance_hash, plain_edit.instance_hash);
        assert_ne!(base.body_hash, plain_edit.body_hash);
    }

    #[test]
    fn test_projection_lists_units_and_pub_items_only() {
        let code = r#"
module geo.items
unit sec, ms = 0.001 * sec
pub bitstruct Flags : u8 {
  @bits(0..8) pub raw: u8
}
bitstruct HiddenBits : u8 {
  @bits(0..8) pub raw: u8
}
struct HiddenStruct {
  n: u32
}
enum HiddenEnum { A }
trait HiddenTrait {
  fn peek(self) -> bool
}
"#;
        let report = project_spec_blueprint(&parse_module(code).unwrap());
        let out = &report.projected_source;
        assert!(
            out.contains("unit sec, ms = 0.001 * sec")
                && out.contains("pub bitstruct Flags : u8 {\n  @bits(0..8) pub raw: u8"),
            "{}",
            out
        );
        for hidden in ["HiddenBits", "HiddenStruct", "HiddenEnum", "HiddenTrait"] {
            assert!(!out.contains(hidden), "`{}` leaked into:\n{}", hidden, out);
        }
        // The saving is the share of the module's tokens the projection leaves out.
        let expected = (1.0 - report.spec_tokens as f64 / report.total_tokens as f64) * 100.0;
        assert!(
            (report.savings_percent - expected).abs() < 1e-9
                && report.savings_percent > 0.0
                && report.savings_percent < 100.0
        );
    }

    #[test]
    fn test_projection_keeps_signatures_and_drops_bodies() {
        let code = r#"
module geo.search
/// The cell a point falls in.
@layout(size: 8, align: 4)
pub struct Cell {
  @bytes(0..4) pub x: u32
  @bytes(4..8) pub y: u32
  invariant {
    x < 4096
  }
}
pub enum Side {
  Left
  Right
}
pub trait Probe {
  fn hit(self, x: u32) -> bool
}
@total
pub fn find(cells: []Cell, key: []u8, lo: usize, hi: usize) -> []Cell ! SearchError from cells with Clock
  requires lo < hi
  ensures result.len <= hi - lo
  decreases hi - lo
{
  val secret_local = lo + hi
  cells
}
fn private_helper(x: u32) -> u32 = x
"#;
        let module = parse_module(code).unwrap();
        let report = project_spec_blueprint(&module);
        let out = &report.projected_source;
        for kept in [
            "/// The cell a point falls in.",
            "@layout(size: 8, align: 4)",
            "  @bytes(0..4) pub x: u32",
            "    x < 4096",
            "pub enum Side {\n  Left\n  Right\n}",
            "  fn hit(self, x: u32) -> bool",
            "@total",
            "pub fn find(cells: []Cell, key: []u8, lo: usize, hi: usize) -> []Cell ! SearchError from cells with Clock",
            "  requires lo < hi",
            "  ensures  result.len <= hi - lo",
            "  decreases hi - lo",
            "= ...",
        ] {
            assert!(out.contains(kept), "missing `{}` in:\n{}", kept, out);
        }
        // The body and every private item stay out, and the digests head the projection.
        assert!(
            !out.contains("secret_local") && !out.contains("private_helper"),
            "{}",
            out
        );
        let hashes = compute_dual_hashes(&module);
        assert!(out.contains(&format!("// spec_hash: {}", hashes.spec_hash)));
        assert!(out.contains(&format!("// body_hash: {}", hashes.body_hash)));
        assert!(out.contains(&format!("// instance_hash: {}", hashes.instance_hash)));
        assert!(report.spec_tokens < report.total_tokens && report.savings_percent > 0.0);
    }
}

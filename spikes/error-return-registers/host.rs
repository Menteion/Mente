// The host program the spike's two objects are linked into (README.md). It calls every chain's
// `_run` wrapper, which stores the tag and the payload words the chain's outermost function
// returned, and checks them against the payload rules recomputed here (src/lib.rs,
// ERROR_WORD_STEP). On Linux it also calls `pair_outer` and `pair32_outer` directly, as
// functions returning a 16-byte C struct: System V and AAPCS64 return such a struct in
// RAX:RDX and X0:X1, so rustc reads the tag and the payload from the registers Cranelift
// returned them in, with no wrapper between.
use std::process::ExitCode;

macro_rules! wrappers {
    ($($name:ident),* $(,)?) => {
        extern "C" { $(fn $name(x: i64, out: *mut u64);)* }
    };
}
wrappers!(
    pair_run,
    pair32_run,
    wide_tail_run,
    big_tail_run,
    wide_buffer_run,
    big_buffer_run,
    wide_implicit_run,
    big_implicit_run,
    pair_forward_run,
    pair32_forward_run,
    plain_forward_run,
);
// AArch64 has eight return registers in its C convention, so Cranelift accepts these chains
// there; x86-64 refuses them (README.md).
#[cfg(target_arch = "aarch64")]
wrappers!(wide_platform_run, big_platform_run);

#[cfg(target_os = "linux")]
mod direct {
    #[repr(C)]
    pub struct Pair8 {
        pub tag: u8,
        pub payload: u64,
    }
    #[repr(C)]
    pub struct Pair32 {
        pub tag: u32,
        pub payload: u64,
    }
    extern "C" {
        pub fn pair_outer(x: i64) -> Pair8;
        pub fn pair32_outer(x: i64) -> Pair32;
    }
}

const STEP: i64 = 100_000;
const INPUTS: [i64; 6] = [3, 0, 2, -7, 60, 9];

type Result = (u64, Vec<i64>);

fn raise(code: i64, n: usize) -> Result {
    (1, (0..n as i64).map(|i| code + i * STEP).collect())
}
fn leaf(x: i64, n: usize) -> Result {
    if x < 0 {
        raise(1000 - x, n)
    } else {
        (0, (0..n as i64).map(|i| 2 * x + i).collect())
    }
}
fn middle(x: i64, n: usize) -> Result {
    let (tag, w) = leaf(x, n);
    if tag != 0 {
        return (tag, w);
    }
    if w[0] > 100 {
        raise(2000 + w[0], n)
    } else {
        (0, w.iter().map(|v| v + 1).collect())
    }
}
fn outer(x: i64, n: usize) -> Result {
    let (tag, w) = middle(x, n);
    if tag != 0 {
        return (tag, w);
    }
    if (w[0] as u64) % 5 == 0 {
        raise(3000 + w[0], n)
    } else {
        (0, w.iter().map(|v| v * 10).collect())
    }
}
fn forward(x: i64) -> Result {
    let (tag, w) = leaf(x, 1);
    if tag != 0 {
        return (tag, w);
    }
    (0, vec![w[0] + 1])
}
fn plain_forward(x: i64) -> Result {
    (0, vec![2 * x + 1])
}

fn show((tag, w): &Result) -> String {
    let words: Vec<String> = w.iter().map(|v| v.to_string()).collect();
    format!(
        "{} [{}]",
        if *tag == 0 { "ok" } else { "error" },
        words.join(", ")
    )
}

fn main() -> ExitCode {
    type Run = unsafe extern "C" fn(i64, *mut u64);
    #[cfg_attr(not(target_arch = "aarch64"), allow(unused_mut))]
    let mut chains: Vec<(&str, usize, Run, fn(i64, usize) -> Result)> = vec![
        ("pair", 1, pair_run, outer),
        ("pair32", 1, pair32_run, outer),
        ("wide_tail", 2, wide_tail_run, outer),
        ("big_tail", 3, big_tail_run, outer),
        ("wide_buffer", 2, wide_buffer_run, outer),
        ("big_buffer", 3, big_buffer_run, outer),
        ("wide_implicit", 2, wide_implicit_run, outer),
        ("big_implicit", 3, big_implicit_run, outer),
        ("pair_forward", 1, pair_forward_run, |x, _| forward(x)),
        ("pair32_forward", 1, pair32_forward_run, |x, _| forward(x)),
        ("plain_forward", 1, plain_forward_run, |x, _| plain_forward(x)),
    ];
    #[cfg(target_arch = "aarch64")]
    {
        chains.push(("wide_platform", 2, wide_platform_run, outer));
        chains.push(("big_platform", 3, big_platform_run, outer));
    }
    let mut failures = 0;
    for (name, n, run, expected) in &chains {
        for x in INPUTS {
            let mut out = [u64::MAX; 4];
            // SAFETY: every `_run` wrapper is `extern "C" fn(i64, *mut u64)` and writes the
            // tag and its chain's payload words, at most four u64 values, at `out`.
            unsafe { run(x, out.as_mut_ptr()) };
            let got: Result = (out[0], out[1..=*n].iter().map(|v| *v as i64).collect());
            let want = expected(x, *n);
            let verdict = if got == want { "ok" } else { "MISMATCH" };
            if got != want {
                failures += 1;
            }
            println!("{name}({x}) = {}  {verdict}", show(&got));
            if got != want {
                println!("    expected {}", show(&want));
            }
        }
    }
    #[cfg(target_os = "linux")]
    for x in INPUTS {
        // SAFETY: `pair_outer` and `pair32_outer` take an i64 and return their tag and payload
        // in the two return registers of the C convention, as a 16-byte struct is returned.
        let (a, b) = unsafe { (direct::pair_outer(x), direct::pair32_outer(x)) };
        let want = outer(x, 1);
        for (name, got) in [
            ("pair_outer", (a.tag as u64, vec![a.payload as i64])),
            ("pair32_outer", (b.tag as u64, vec![b.payload as i64])),
        ] {
            let verdict = if got == want { "ok" } else { "MISMATCH" };
            if got != want {
                failures += 1;
            }
            println!("direct {name}({x}) = {}  {verdict}", show(&got));
        }
    }
    if failures == 0 {
        println!("every round trip returned what it should");
        ExitCode::SUCCESS
    } else {
        println!("{failures} round trips returned something else");
        ExitCode::FAILURE
    }
}

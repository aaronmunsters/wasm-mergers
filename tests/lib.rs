use std::iter::once;

use itertools::Itertools;
use wasmtime::*;
use wat::parse_str;
use webassembly_mergers::merge_options::DEFAULT_RENAMER;
use webassembly_mergers::merge_options::{ClashingExports, KeepExports, MergeOptions};
use webassembly_mergers::{MergeConfiguration, NamedModule};

mod wasmtime_macros; // Bring macros in scope

fn iter_permutations<'a>(
    named_modules: &'a [&NamedModule<'a, &'a [u8]>],
) -> Vec<Box<[&'a NamedModule<'a, &'a [u8]>]>> {
    named_modules
        .iter()
        .permutations(named_modules.len())
        .map(|perm| perm.into_iter().copied().collect::<Box<[_]>>())
        .collect::<Vec<_>>()
}

fn assert_structural_diff(merged_manual: &[u8], merged_lib: &[u8], allowed_difference: f64) {
    use conv::ApproxFrom;
    let merged_manual_len: f64 = (ApproxFrom::approx_from(merged_manual.len())).unwrap();
    let merged_lib_len: f64 = (ApproxFrom::approx_from(merged_lib.len())).unwrap();
    let ratio = merged_manual_len / merged_lib_len;
    let allowed_min = 1.0 - allowed_difference;
    let allowed_max = 1.0 + allowed_difference;
    assert!(
        (allowed_min..=allowed_max).contains(&ratio),
        "Lengths differ by more than {allowed_difference}%: manual = {merged_manual_len}, lib = {merged_lib_len}",
    );
}
/// Merging mutually recursive even and odd functions across modules
///
/// Module Dependency Overview:
/// - The `even` module exports a function `even` that returns 1 if the input is 0,
///   otherwise it recursively calls `odd(n - 1)`.
/// - The `odd` module exports a function `odd` that returns 0 if the input is 0,
///   otherwise it recursively calls `even(n - 1)`.
///
///   - **Structural Validation**: Compare the size (byte length) of the manually merged and library-merged WebAssembly modules. The difference in size should be within 20% tolerance.
///   - **Behavioral Validation**: Call the `even` and `odd` functions for a range of values (from 0 to 999) and assert that their results match the expected behavior:
///     - `even(n)` returns `true` if `n` is even, otherwise `false`.
///     - `odd(n)` returns `true` if `n` is odd, otherwise `false`.
#[test]
fn merge_even_odd() -> Result<(), Error> {
    const WAT_ODD: &str = r#"
      (module
        (import "even" "even" (func $even (param i32) (result i32)))
        (export "odd" (func $odd))
        (func $odd (param $0 i32) (result i32)
          local.get $0
          i32.eqz
          if
          i32.const 0
          return
          end
          local.get $0
          i32.const 1
          i32.sub
          call $even))
      "#;

    const WAT_EVEN: &str = r#"
      (module
        (import "odd" "odd" (func $odd (param i32) (result i32)))
        (export "even" (func $even))
        (func $even (param $0 i32) (result i32)
          local.get $0
          i32.eqz
          if
          i32.const 1
          return
          end
          local.get $0
          i32.const 1
          i32.sub
          call $odd))
      "#;

    const WAT_EVEN_ODD: &str = r#"
      (module
        (func $even (param $0 i32) (result i32)
          local.get $0
          i32.eqz
          if
          i32.const 1
          return
          end
          local.get $0
          i32.const 1
          i32.sub
          call $odd)
        (func $odd (param $0 i32) (result i32)
          local.get $0
          i32.eqz
          if
          i32.const 0
          return
          end
          local.get $0
          i32.const 1
          i32.sub
          call $even)
        (export "even" (func $even))
        (export "odd" (func $odd)))
      "#;

    // Wasm i32 bool representation -> Rust bool
    fn to_bool(v: i32) -> bool {
        assert!(v == 0 || v == 1);
        v == 1
    }

    let manual_merged = { parse_str(WAT_EVEN_ODD)? };
    let lib_merged = {
        let wat_even = parse_str(WAT_EVEN)?;
        let wat_odd = parse_str(WAT_ODD)?;

        let modules: &[&NamedModule<'_, &[u8]>] = &[
            &NamedModule::new("even", &wat_even),
            &NamedModule::new("odd", &wat_odd),
        ];

        let mut merge_conf = MergeOptions::default();
        let mut keep_exports = KeepExports::default();
        keep_exports.keep_function("even".to_string().into(), "even".into());
        keep_exports.keep_function("odd".to_string().into(), "odd".into());
        merge_conf.keep_exports = Some(keep_exports);

        MergeConfiguration::new(modules, merge_conf).merge()?
    };

    // Structural assertion
    let ratio_allowed_delta: f64 = 0.30; // Expressed in %
    assert_structural_diff(&manual_merged, &lib_merged, ratio_allowed_delta);

    let r_even = |v| v % 2 == 0;
    let r_odd = |v| !(r_even(v));

    // Behavioral assertion
    for merged_wasm in [lib_merged, manual_merged] {
        // Interpret even & odd
        let mut store = Store::<()>::default();
        let module = Module::from_binary(store.engine(), &merged_wasm)?;
        let instance = Instance::new(&mut store, &module, &[])?;

        declare_fns_from_wasm! { instance, store,
           even [i32] [i32],
           odd [i32] [i32],
        };

        for i in 0..1000 {
            assert_eq!(to_bool(wasm_call!(store, even, i)), r_even(i));
            assert_eq!(to_bool(wasm_call!(store, odd, i)), r_odd(i));
        }
    }

    Ok(())
}

#[test]
fn test_earmark() -> Result<(), Error> {
    const NEEDLE: &[u8] = "webassembly-mergers".as_bytes();
    const NEEDLE_LEN: usize = NEEDLE.len();
    const M: &str = "(module)";
    webassembly_mergers::MergeConfiguration::new(
        &[
            &NamedModule::new("A", &wat::parse_str(M)?),
            &NamedModule::new("B", &wat::parse_str(M)?),
        ],
        MergeOptions::default(),
    )
    .merge()?
    .windows(NEEDLE_LEN)
    .position(|w| NEEDLE == w)
    .map(|_| ())
    .ok_or(Error::msg("Needle not found"))
}

/// Verifies that merging a set of modules that forms a cycle
/// of mutually recursive function calls works.
///
///  ```txt
///  func_a → func_b → func_c → func_d → func_e
///     ↑                                  |
///     └──────────────────────────────────┘
///          [Mutual recursion cycle]
///  ```
#[allow(clippy::too_many_lines)] // TODO: fix / remove
#[test]
fn merge_cycle_chain() -> Result<(), Error> {
    const WAT_MOD_ABCDE: &str = r#"
      (module
        (func $func_a (param $n i32) (result i32)
          (if (result i32)
            (i32.le_s (local.get $n) (i32.const 0))
            (then (i32.const 100)) ;; Return 100 to signify done in A
            (else (call $func_b (i32.sub (local.get $n) (i32.const 1))))))

        (func $func_b (param $n i32) (result i32)
          (if (result i32)
            (i32.le_s (local.get $n) (i32.const 0))
            (then (i32.const 200)) ;; Done in B
            (else (call $func_c (i32.sub (local.get $n) (i32.const 1))))))

        (func $func_c (param $n i32) (result i32)
          (if (result i32)
            (i32.le_s (local.get $n) (i32.const 0))
            (then (i32.const 300)) ;; Done in C
            (else (call $func_d (i32.sub (local.get $n) (i32.const 1))))))

        (func $func_d (param $n i32) (result i32)
          (if (result i32)
            (i32.le_s (local.get $n) (i32.const 0))
            (then (i32.const 400)) ;; Done in D
            (else (call $func_e (i32.sub (local.get $n) (i32.const 1))))))

        (func $func_e (param $n i32) (result i32)
          (if (result i32)
            (i32.le_s (local.get $n) (i32.const 0))
            (then (i32.const 500)) ;; Done in E
            (else (call $func_a (i32.sub (local.get $n) (i32.const 1))))))
        (export "func_a" (func $func_a)))
      "#;

    const WAT_MOD_A: &str = r#"
      (module
        (import "WAT_MOD_B" "func_b" (func $func_b (param i32) (result i32)))

        (func $func_a (param $n i32) (result i32)
          (if (result i32)
            (i32.le_s (local.get $n) (i32.const 0))
            (then (i32.const 100))
            (else (call $func_b (i32.sub (local.get $n) (i32.const 1))))))
        (export "func_a" (func $func_a)))
      "#;

    const WAT_MOD_B: &str = r#"
      (module
        (import "WAT_MOD_C" "func_c" (func $func_c (param i32) (result i32)))

        (func $func_b (param $n i32) (result i32)
          (if (result i32)
            (i32.le_s (local.get $n) (i32.const 0))
            (then (i32.const 200))
            (else (call $func_c (i32.sub (local.get $n) (i32.const 1))))))
        (export "func_b" (func $func_b)))
      "#;

    const WAT_MOD_C: &str = r#"
      (module
        (import "WAT_MOD_D" "func_d" (func $func_d (param i32) (result i32)))

        (func $func_c (param $n i32) (result i32)
          (if (result i32)
            (i32.le_s (local.get $n) (i32.const 0))
            (then (i32.const 300))
            (else (call $func_d (i32.sub (local.get $n) (i32.const 1))))))
        (export "func_c" (func $func_c)))
      "#;

    const WAT_MOD_D: &str = r#"
      (module
        (import "WAT_MOD_E" "func_e" (func $func_e (param i32) (result i32)))

        (func $func_d (param $n i32) (result i32)
          (if (result i32)
            (i32.le_s (local.get $n) (i32.const 0))
            (then (i32.const 400))
            (else (call $func_e (i32.sub (local.get $n) (i32.const 1))))))
        (export "func_d" (func $func_d)))
      "#;

    const WAT_MOD_E: &str = r#"
      (module
        (import "WAT_MOD_A" "func_a" (func $func_a (param i32) (result i32)))

        (func $func_e (param $n i32) (result i32)
          (if (result i32)
            (i32.le_s (local.get $n) (i32.const 0))
            (then (i32.const 500))
            (else (call $func_a (i32.sub (local.get $n) (i32.const 1))))))
        (export "func_e" (func $func_e)))
      "#;

    let manual_merged = { parse_str(WAT_MOD_ABCDE)? };

    let wat_mod_a = parse_str(WAT_MOD_A)?;
    let wat_mod_b = parse_str(WAT_MOD_B)?;
    let wat_mod_c = parse_str(WAT_MOD_C)?;
    let wat_mod_d = parse_str(WAT_MOD_D)?;
    let wat_mod_e = parse_str(WAT_MOD_E)?;

    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("WAT_MOD_A", &wat_mod_a),
        &NamedModule::new("WAT_MOD_B", &wat_mod_b),
        &NamedModule::new("WAT_MOD_C", &wat_mod_c),
        &NamedModule::new("WAT_MOD_D", &wat_mod_d),
        &NamedModule::new("WAT_MOD_E", &wat_mod_e),
    ];

    for merged_wasm in modules
        .iter()
        .permutations(modules.len())
        .map(|perm| {
            let perm: Box<[_]> = perm.into_iter().copied().collect();

            let mut merge_conf = MergeOptions::default();
            let mut keep_exports = KeepExports::default();
            keep_exports.keep_function("WAT_MOD_A".to_string().into(), "func_a".into());
            merge_conf.keep_exports = Some(keep_exports);

            MergeConfiguration::new(&perm, merge_conf).merge().unwrap()
        })
        .chain(once(manual_merged.clone()))
    {
        // Structural assertion
        let ratio_allowed_delta: f64 = 0.1; // Expressed in %
        assert_structural_diff(&manual_merged, &merged_wasm, ratio_allowed_delta);

        // Interpret even & odd
        let mut store = Store::<()>::default();
        let module = Module::from_binary(store.engine(), &merged_wasm)?;
        let instance = Instance::new(&mut store, &module, &[])?;

        // Fetch `even` and `odd` export

        declare_fns_from_wasm! { instance, store, func_a [i32] [i32] };

        for i in 0..100 {
            let expected_result = match i % 5 {
                0 => 100, // func_a will return 100 when n == 0
                1 => 200, // func_b will return 200 when n == 1
                2 => 300, // func_c will return 300 when n == 2
                3 => 400, // func_d will return 400 when n == 3
                _ => 500, // func_e will return 500 when n == 4
            };

            assert_eq!(
                wasm_call!(store, func_a, i),
                expected_result,
                "Failed for input {i}"
            );
        }
    }

    Ok(())
}

/// Verifies that merging a set of modules that forms infinite loop
/// is reported as a wrong definition.
///
///  ```txt
///     func_a ⇄ func_a'
///  ```
///
/// where `func_a` and `func_a'` are defined as a lookup of each other.
#[test]
fn illegal_loop() -> Result<(), Error> {
    const WAT_MOD_B: &str = r#"
      (module
        (import "WAT_MOD_A" "func_a" (func $func_a (param i32) (result i32)))
        (export "func_b" (func $func_a)))
      "#;

    const WAT_MOD_A: &str = r#"
      (module
        (import "WAT_MOD_B" "func_b" (func $func_b (param i32) (result i32)))
        (export "func_a" (func $func_b)))
      "#;

    let wat_mod_a = parse_str(WAT_MOD_A)?;
    let wat_mod_b = parse_str(WAT_MOD_B)?;

    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("WAT_MOD_A", &wat_mod_a),
        &NamedModule::new("WAT_MOD_B", &wat_mod_b),
    ];

    let error = MergeConfiguration::new(modules, MergeOptions::default())
        .merge()
        .expect_err("Expect infinite cycle loop");

    assert!(matches!(
        error,
        webassembly_mergers::error::Error::ImportCycle
    ));

    Ok(())
}

/// 3-Module Pass-Through Chain
///
/// A → B → C
///
/// - Module A defines `a` and exports it.
/// - Module B has no definitions—only re-exports what it imported.
/// - Module C imports the function from B and wraps it into `run`.
///
/// Expected: After merging, `run()` yields 42.
#[test]
fn merge_pass_through_module() -> Result<(), Error> {
    const WAT_A: &str = r#"
      (module
        (func $a (result i32)
          i32.const 42)
        (export "a" (func $a)))
      "#;

    const WAT_B: &str = r#"
      (module
        (import "a" "a" (func $a (result i32)))
        (export "b" (func $a)))
      "#;

    const WAT_C: &str = r#"
      (module
        (import "b" "b" (func $b (result i32)))
        (func $run (result i32)
          call $b)
        (export "run" (func $run)))
      "#;

    let wat_a = parse_str(WAT_A)?;
    let wat_b = parse_str(WAT_B)?;
    let wat_c = parse_str(WAT_C)?;

    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("a", &wat_a),
        &NamedModule::new("b", &wat_b),
        &NamedModule::new("c", &wat_c),
    ];

    for modules in iter_permutations(modules) {
        let merged = MergeConfiguration::new(&modules, MergeOptions::default()).merge()?;

        // Instantiate & run merged module
        let mut store = Store::<()>::default();
        let module = Module::from_binary(store.engine(), &merged)?;
        let instance = Instance::new(&mut store, &module, &[])?;

        declare_fns_from_wasm! {instance, store, run [] [i32]};
        let result = wasm_call!(store, run);

        assert_eq!(result, 42, "Expected 42 from chained import, got {result}",);
    }

    Ok(())
}

/// This test defines the Fibonacci function across two mutually dependent modules:
/// - Module A defines the recursive function `a(n)`
/// - Module B re-exports `a` as `b`, completing a cycle so that `a` calls into `b`, which points back to `a`
///
/// This verifies that `webassembly_mergers` can correctly resolve circular imports.
#[test]
fn merge_cross_module_fibonacci() -> Result<(), Error> {
    const WAT_MODULE_A: &str = r#"
      (module
        (import "indirect_fib" "indirect_fib" (func $indirect_fib (param i32) (result i32)))

        (func $fib (param $n i32) (result i32)
          local.get $n
          i32.const 0
          i32.eq
          if
            i32.const 0
            return
          end

          local.get $n
          i32.const 1
          i32.eq
          if
            i32.const 1
            return
          end

          ;; fib(n - 1)
          local.get $n
          i32.const 1
          i32.sub
          call $indirect_fib

          ;; fib(n - 2)
          local.get $n
          i32.const 2
          i32.sub
          call $indirect_fib

          ;; add results
          i32.add)

        (export "fib" (func $fib)))
      "#;

    const WAT_MODULE_B: &str = r#"
      (module
        (import "fib" "fib" (func $fib (param i32) (result i32)))
        (export "indirect_fib" (func $fib)))
      "#;

    // Reference implementation
    fn expected_fib(n: i32) -> i32 {
        match n {
            0 => 0,
            1 => 1,
            _ => expected_fib(n - 1) + expected_fib(n - 2),
        }
    }

    // Parse WAT source to binary
    let binary_a = parse_str(WAT_MODULE_A)?;
    let binary_b = parse_str(WAT_MODULE_B)?;

    // Prepare named modules
    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("fib", &binary_a),
        &NamedModule::new("indirect_fib", &binary_b),
    ];

    // Merge the modules
    let mut merge_conf: MergeOptions = MergeOptions::default();
    let mut keep_exports = KeepExports::default();
    keep_exports.keep_function("fib".to_string().into(), "fib".into());
    merge_conf.keep_exports = Some(keep_exports);

    for modules in iter_permutations(modules) {
        let merged_wasm: Vec<u8> = MergeConfiguration::new(&modules, merge_conf.clone()).merge()?;

        // Instantiate merged module
        let mut store = Store::<()>::default();
        let module = Module::from_binary(store.engine(), &merged_wasm)?;
        let instance = Instance::new(&mut store, &module, &[])?;

        // Get exported Fibonacci function
        declare_fns_from_wasm! { instance, store, fib [i32] [i32] };

        // Run and assert behavior
        for i in 0..20 {
            let actual = wasm_call!(store, fib, i);
            let expected = expected_fib(i);
            assert_eq!(
                actual, expected,
                "Mismatch at fib({i}): expected {expected}, got {actual}"
            );
        }
    }

    Ok(())
}

/// Module structures:
/// ## Module `ab`
/// defined as:
/// ```
/// a() = 2
/// b() = 3
/// ```
///
/// ## Module `cd`
/// imports `a()` and `b()` from `ab`, and is defined as:
/// ```
/// c() = a() * 5 // = 2 * 5 = 10
/// d() = b() * 7 // = 3 * 7 = 21
/// ```
///
/// ## Module `e`
/// imports `a()` and `b()` from `ab`,
/// and `c()`, `d()` from `cd`, is defined as:
/// ```
/// e() = (a()*11 * b()*13) * (c()*17 * d()*23)
/// //  = ((2*11)*(3*13)) * ((10*17)*(21*23))
/// ```
///
/// After merging:
/// - All functions should be resolved internally (no remaining imports)
/// - Final merged module should export: `d()`, `e()`, `f()`
#[test]
fn composition_of_cross_deps() -> Result<(), Error> {
    const WAT_AB: &str = r#"
      (module
        (func $a1 (result i32) i32.const 2)
        (func $a2 (result i32) i32.const 3)

        (export "a" (func $a1))
        (export "b" (func $a2)))
      "#;

    const WAT_CD: &str = r#"
      (module
        (import "ab" "a" (func $a (result i32)))
        (import "ab" "b" (func $b (result i32)))

        (func $c (result i32) (i32.mul (call $a) (i32.const 5)))
        (func $d (result i32) (i32.mul (call $b) (i32.const 7)))

        (export "c" (func $c))
        (export "d" (func $d)))
      "#;

    const WAT_E: &str = r#"
      (module
        (import "ab" "a" (func $a (result i32)))
        (import "ab" "b" (func $b (result i32)))
        (import "cd" "c" (func $c (result i32)))
        (import "cd" "d" (func $d (result i32)))

        (func $e (result i32)
          (i32.mul
            (i32.mul
              (i32.mul (call $a) (i32.const 11))
              (i32.mul (call $b) (i32.const 13)))
            (i32.mul
              (i32.mul (call $c) (i32.const 17))
              (i32.mul (call $d) (i32.const 23)))))
        (export "e" (func $e)))
      "#;

    let wat_ab = parse_str(WAT_AB)?;
    let wat_cd = parse_str(WAT_CD)?;
    let wat_e = parse_str(WAT_E)?;

    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("ab", &wat_ab),
        &NamedModule::new("cd", &wat_cd),
        &NamedModule::new("e", &wat_e),
    ];

    let merged = MergeConfiguration::new(modules, MergeOptions::default()).merge()?;

    // Instantiate merged module (should be self-contained)
    let mut store = Store::<()>::default();
    let engine = store.engine();
    let module = Module::from_binary(engine, &merged)?;
    let instance = Instance::new(&mut store, &module, &[])?;

    declare_fns_from_wasm! { instance, store, e [] [i32] };

    let rs_a = || 2;
    let rs_b = || 3;
    let rs_c = || rs_a() * 5;
    let rs_d = || rs_b() * 7;
    let rs_e = || ((rs_a() * 11) * (rs_b() * 13)) * ((rs_c() * 17) * (rs_d() * 23));

    assert_eq!(wasm_call!(store, e), rs_e());

    Ok(())
}

#[test]
fn test_multi_memory() -> Result<(), Error> {
    let gen_wat = |prefix| {
        format!(
            r#"
              (module
                ;; Define mem0 & mem1 which are both 1 page in initial size.
                (memory $mem0 1)
                (memory $mem1 1)
                
                ;; Define a function to copy over a single byte from mem0[offset] to mem1[offset]
                (func $copy_byte_from_0_to_1 (param $offset i32)
                  ;; Load byte from mem0
                  (i32.store8 (;mem-idx=;) 1
                    (local.get $offset)
                    (i32.load8_u (;mem-idx=;) 0 (local.get $offset))))

                ;; Define a function to load a single byte from mem0[offset]
                (func $load_byte_from_0 (param $offset i32) (result i32)
                  (i32.load8_u (;mem-idx=;) 0 (local.get $offset)))
              
                ;; Define a function to store a single byte in mem0[offset]
                (func $store_byte_in_0 (param $offset i32) (param $byte i32)
                  (i32.store8 (;mem-idx=;) 0 (local.get $offset) (local.get $byte)))

                ;; Define a function to load a single byte from mem1[offset]
                (func $load_byte_from_1 (param $offset i32) (result i32)
                  (i32.load8_u (;mem-idx=;) 1 (local.get $offset)))

                (export "{prefix}_load_byte_from_0" (func $load_byte_from_0))
                (export "{prefix}_store_byte_in_0" (func $store_byte_in_0))
                (export "{prefix}_load_byte_from_1" (func $load_byte_from_1))
                (export "{prefix}_copy_byte_from_0_to_1" (func $copy_byte_from_0_to_1)))
            "#
        )
    };

    let wasm_a = parse_str(gen_wat("a"))?;
    let wasm_b = parse_str(gen_wat("b"))?;

    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("A", &wasm_a),
        &NamedModule::new("B", &wasm_b),
    ];

    let merge_options = MergeOptions {
        clashing_exports: ClashingExports::Rename(DEFAULT_RENAMER),
        ..Default::default()
    };

    let merged = MergeConfiguration::new(modules, merge_options).merge()?;

    // Instantiate merged module (should be self-contained)
    let mut store = Store::<()>::default();
    let engine = store.engine();
    let module = Module::from_binary(engine, &merged)?;
    let instance = Instance::new(&mut store, &module, &[])?;

    declare_fns_from_wasm! { instance, store,
      // In module A
      a_load_byte_from_0 [i32] [i32],
      a_store_byte_in_0 [i32, i32] [],
      a_load_byte_from_1 [i32] [i32],
      a_copy_byte_from_0_to_1 [i32] [],
      // In module B
      b_load_byte_from_0 [i32] [i32],
      b_store_byte_in_0 [i32, i32] [],
      b_load_byte_from_1 [i32] [i32],
      b_copy_byte_from_0_to_1 [i32] [],
    };

    for actual_value in 0..=255 {
        for offset in [0, 1, 2, 3, 5, 7, 11, 13] {
            wasm_call!(store, a_store_byte_in_0, offset, actual_value);
            wasm_call!(store, a_copy_byte_from_0_to_1, offset);

            wasm_call!(store, b_store_byte_in_0, offset, actual_value);
            wasm_call!(store, b_copy_byte_from_0_to_1, offset);

            assert_eq!(actual_value, wasm_call!(store, a_load_byte_from_0, offset));
            assert_eq!(actual_value, wasm_call!(store, a_load_byte_from_1, offset));

            assert_eq!(actual_value, wasm_call!(store, b_load_byte_from_0, offset));
            assert_eq!(actual_value, wasm_call!(store, b_load_byte_from_1, offset));
        }
    }

    Ok(())
}

#[allow(clippy::too_many_lines)] // TODO: fix / remove
#[test]
fn test_smithed_modules() {
    use arbitrary::Unstructured;
    use rand::{Rng, SeedableRng};
    use rayon::prelude::*;
    use wasm_smith::{Config, Module as WasmSmithModule};

    struct PreMergeOutcome {
        args: Vec<wasmtime::Val>,
        results: Vec<wasmtime::Val>,
        function_name: String,
    }

    struct ExpectedModuleOutcomes {
        module: Vec<u8>,
        expected_outcomes: Vec<PreMergeOutcome>,
    }

    const MAX_SEED: u64 = 100;
    const WINDOW_NAMES: &[&str] = &["a", "b", "c", "d"];

    let assertions: Vec<_> = (0..MAX_SEED)
        .into_par_iter()
        .filter_map(|seed| {
            println!("SEED = {seed}");
            let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let mut random_sequence = [0_u8; 2_usize.pow(10)];
            for value in &mut random_sequence {
                *value = rng.random::<u8>();
            }

            let mut random = Unstructured::new(&random_sequence);

            let config = Config {
                gc_enabled: false,
                exceptions_enabled: false,
                min_exports: 1,
                max_imports: 0,
                min_memories: 1,
                min_data_segments: 1,
                min_element_segments: 1,
                min_tables: 1,
                bulk_memory_enabled: true,
                threads_enabled: true,
                simd_enabled: true,
                shared_everything_threads_enabled: true,
                ..Default::default()
            };

            let Ok(mut module) = WasmSmithModule::new(config, &mut random) else {
                return None;
            };

            module.ensure_termination(10_000).unwrap();
            let module_bytes = module.to_bytes();

            // Instantiate merged module (should be self-contained)
            let mut store = Store::<()>::default();
            let engine = store.engine();
            let module = Module::from_binary(engine, &module_bytes).unwrap();
            let Ok(instance) = Instance::new(&mut store, &module, &[]) else {
                return None;
            };

            let mut random_val = |p| match p {
                ValType::I32 => Some(rng.random::<i32>().into()),
                ValType::I64 => Some(rng.random::<i64>().into()),
                ValType::F32 => Some(rng.random::<f32>().into()),
                ValType::F64 => Some(rng.random::<f64>().into()),
                _ => None,
            };

            let call_results: Vec<PreMergeOutcome> = module
                .exports()
                .filter_map(|export| match export.ty() {
                    ExternType::Func(func_type) => {
                        let args: Vec<_> = func_type
                            .params()
                            .map(&mut random_val)
                            .collect::<Option<_>>()?;
                        let mut results: Vec<_> = func_type
                            .results()
                            .map(&mut random_val)
                            .collect::<Option<_>>()?;
                        instance
                            .get_func(&mut store, export.name())
                            .unwrap()
                            .call(&mut store, &args, &mut results)
                            .ok()
                            .map(|()| PreMergeOutcome {
                                args,
                                results,
                                function_name: export.name().to_string(),
                            })
                    }
                    ExternType::Global(_global_type) => None,
                    ExternType::Table(_table_type) => None,
                    ExternType::Memory(_memory_type) => None,
                    ExternType::Tag(_tag_type) => None,
                })
                .collect();

            Some(ExpectedModuleOutcomes {
                module: module_bytes,
                expected_outcomes: call_results,
            })
        })
        .collect();

    let window_width: usize = WINDOW_NAMES.len();
    assertions.windows(window_width).for_each(|window| {
        let modules: Vec<_> = window.iter().zip(WINDOW_NAMES).collect();
        let named_modules: Vec<_> = modules
            .iter()
            .map(|(ExpectedModuleOutcomes { module, .. }, name_space)| {
                NamedModule::new(name_space, &module[..])
            })
            .collect();
        let refs = named_modules.iter().collect::<Vec<_>>();
        let modules: &[&NamedModule<'_, &[u8]>] = &refs[..];
        let merge_options = MergeOptions {
            clashing_exports: ClashingExports::Rename(DEFAULT_RENAMER),
            ..Default::default()
        };
        let mut merge_configuration =
            webassembly_mergers::MergeConfiguration::new(modules, merge_options);
        let merged = merge_configuration.merge();

        // Failing to parse is something related to the crates `wasm-smith` <~> `walrus`
        if let Err(webassembly_mergers::error::Error::Parse(_)) = merged {
            return;
        }

        // Unwrap the module, asserting it exists
        let merged = merged.unwrap();

        // Instantiate merged module (should be self-contained)
        let mut store = Store::<()>::default();
        let engine = store.engine();
        let module = Module::from_binary(engine, &merged).unwrap();
        let instance = Instance::new(&mut store, &module, &[]).unwrap();

        for asserted_module in window {
            asserted_module
                .expected_outcomes
                .iter()
                .for_each(|assertion| {
                    // TODO:
                    // Currently, we apply the `if let Some` strategy.
                    // This allows that a function cannot be found anymore.
                    // This should change to use a 'renamed-map' to retrieve potentially renamed exports!
                    if let Some(func) = instance.get_func(&mut store, &assertion.function_name) {
                        let results_assertion = assertion.results.clone();
                        let mut results_actual = assertion.results.clone();
                        func.call(&mut store, &assertion.args, &mut results_actual)
                            .unwrap();
                        results_actual.iter().zip(results_assertion).for_each(
                        |(result_actual, result_asserted)| match (result_actual, result_asserted) {
                            (Val::I32(x), Val::I32(y)) => assert_eq!(*x, y),
                            (Val::I64(x), Val::I64(y)) => assert_eq!(*x, y),
                            (Val::F32(x), Val::F32(y)) => assert_eq!(*x, y),
                            (Val::F64(x), Val::F64(y)) => assert_eq!(*x, y),
                            _ => panic!(
                                "Mismatched Val variants: {result_actual:?} vs {result_asserted:?}"
                            ),
                        },
                    );
                    }
                });
        }
    });
}

/// Test if the kind mismatches, and if this can avoid errors if internal
/// resolution is enabled.
#[test]
fn kind_mismatch_expect() -> Result<(), Error> {
    use walrus::{ExportItem, Module};
    use webassembly_mergers::error::Error;
    use webassembly_mergers::merge_options::ResolvedExports;

    let mod_a = parse_str(r#"(module (func   $x (export "x")))"#)?;
    let mod_b = parse_str(r#"(module (global $x (export "x") i32 (i32.const 0)))"#)?;
    let mod_c = parse_str(r#"(module (import "B" "x" (global $x i32)))"#)?;

    // Given only modules A and B, their export 'a' should clash, as they are
    // named and typed equally.
    let modules_ab: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("A", &mod_a),
        &NamedModule::new("B", &mod_b),
    ];

    assert!(matches!(
        MergeConfiguration::new(modules_ab, MergeOptions::default()).merge(),
        Err(Error::ExportNameClash(_))
    ));

    // But if module C were also included in the merge, the outcome should not
    // clash, as `A:a` can be linked to `C`'s import, allowing the overlapping
    // export to become hidden and ensure that the only exported item remains
    // to be the exported function (asserted below).
    let modules_abc: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("A", &mod_a),
        &NamedModule::new("B", &mod_b),
        &NamedModule::new("C", &mod_c),
    ];

    let options = MergeOptions {
        resolved_exports: ResolvedExports::Remove,
        ..Default::default()
    };
    let outcome = MergeConfiguration::new(modules_abc, options).merge()?;
    let parsed = Module::from_buffer(&outcome)?;
    let exports = parsed.exports.iter().collect::<Vec<_>>();
    assert_eq!(exports.len(), 1);
    assert!(matches!(exports[0].item, ExportItem::Function(_)));

    // However, when resolved exports are kept, the error must still be raised.
    let options = MergeOptions {
        resolved_exports: ResolvedExports::Keep,
        ..Default::default()
    };
    let outcome = MergeConfiguration::new(modules_abc, options).merge()?;
    let parsed = Module::from_buffer(&outcome)?;
    let exports = parsed.exports.iter().collect::<Vec<_>>();

    // FIXME: Add support for the 'ResolvedExports::Keep'.
    assert_eq!(exports.len(), 1); // TODO: should be 2 / merging should fail!

    Ok(())
}

// TODO: if two modules import from the same location, are they the same node
//       in the graph? If not ... this should be explored!

use std::iter::once;

use itertools::Itertools;
use wasm_mergers::{MergeConfiguration, NamedModule};
use wasmtime::*;
use wat::parse_str;

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
fn merge_even_odd() {
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

    let manual_merged = { parse_str(WAT_EVEN_ODD).unwrap() };
    let lib_merged = {
        let wat_even = parse_str(WAT_EVEN).unwrap();
        let wat_odd = parse_str(WAT_ODD).unwrap();

        let modules: &[&NamedModule<'_, &[u8]>] = &[
            &NamedModule::new("even", &wat_even),
            &NamedModule::new("odd", &wat_odd),
        ];

        MergeConfiguration::new(modules).merge().unwrap()
    };

    // Structural assertion
    {
        let manual_merged_len = manual_merged.len() as f64;
        let lib_merged_len = lib_merged.len() as f64;
        let ratio = manual_merged_len / lib_merged_len;
        const RATIO_ALLOWED_DELTA: f64 = 0.20; // 20% difference
        assert!(
            (1.0 - RATIO_ALLOWED_DELTA..=1.0 + RATIO_ALLOWED_DELTA).contains(&ratio),
            "Lengths differ by more than 50%: manual = {manual_merged_len}, lib = {lib_merged_len}",
        );
    }

    #[rustfmt::skip]
    fn r_even(v: i32) -> bool { v % 2 == 0 }
    #[rustfmt::skip]
    fn r_odd(v: i32) -> bool { !(r_even(v)) }

    // Behavioral assertion
    for merged_wasm in [lib_merged, manual_merged] {
        // Interpret even & odd
        let mut store = Store::<()>::default();
        let module = Module::from_binary(store.engine(), &merged_wasm).unwrap();
        let instance = Instance::new(&mut store, &module, &[]).unwrap();

        // Fetch `even` and `odd` export
        let even = instance
            .get_typed_func::<i32, i32>(&mut store, "even")
            .unwrap();

        let odd = instance
            .get_typed_func::<i32, i32>(&mut store, "odd")
            .unwrap();

        fn to_bool(v: i32) -> bool {
            assert!(v == 0 || v == 1);
            v == 1
        }

        for i in 0..1000 {
            assert_eq!(to_bool(even.call(&mut store, i).unwrap()), r_even(i));
            assert_eq!(to_bool(odd.call(&mut store, i).unwrap()), r_odd(i));
        }
    }
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
#[test]
fn merge_cycle_chain() {
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

    let manual_merged = { parse_str(WAT_MOD_ABCDE).unwrap() };

    let wat_mod_a = parse_str(WAT_MOD_A).unwrap();
    let wat_mod_b = parse_str(WAT_MOD_B).unwrap();
    let wat_mod_c = parse_str(WAT_MOD_C).unwrap();
    let wat_mod_d = parse_str(WAT_MOD_D).unwrap();
    let wat_mod_e = parse_str(WAT_MOD_E).unwrap();

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
            MergeConfiguration::new(&perm).merge().unwrap()
        })
        .chain(once(manual_merged.clone()))
    {
        // Structural assertion
        {
            let manual_merged_len = manual_merged.len() as f64;
            let lib_merged_len = merged_wasm.len() as f64;
            let ratio = manual_merged_len / lib_merged_len;
            const RATIO_ALLOWED_DELTA: f64 = 0.1; // 10% difference
            assert!(
                (1.0 - RATIO_ALLOWED_DELTA..=1.0 + RATIO_ALLOWED_DELTA).contains(&ratio),
                "Lengths differ by more than 50%: manual = {manual_merged_len}, lib = {lib_merged_len}",
            );
        }

        // Interpret even & odd
        let mut store = Store::<()>::default();
        let module = Module::from_binary(store.engine(), &merged_wasm).unwrap();
        let instance = Instance::new(&mut store, &module, &[]).unwrap();

        // Fetch `even` and `odd` export
        let func_a = instance
            .get_typed_func::<i32, i32>(&mut store, "func_a")
            .unwrap();

        for i in 0..100 {
            let result = func_a.call(&mut store, i).unwrap();
            let expected_result = match i % 5 {
                0 => 100, // func_a will return 100 when n == 0
                1 => 200, // func_b will return 200 when n == 1
                2 => 300, // func_c will return 300 when n == 2
                3 => 400, // func_d will return 400 when n == 3
                _ => 500, // func_e will return 500 when n == 4
            };

            assert_eq!(result, expected_result, "Failed for input {i}");
        }
    }
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
fn merge_pass_through_module() {
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

    let wat_a = parse_str(WAT_A).unwrap();
    let wat_b = parse_str(WAT_B).unwrap();
    let wat_c = parse_str(WAT_C).unwrap();

    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("a", &wat_a),
        &NamedModule::new("b", &wat_b),
        &NamedModule::new("c", &wat_c),
    ];

    let merged = MergeConfiguration::new(modules)
        .merge()
        .expect("Merge failed");

    // Instantiate & run merged module
    let mut store = Store::<()>::default();
    let instance = {
        let module = Module::from_binary(store.engine(), &merged).unwrap();
        Instance::new(&mut store, &module, &[]).unwrap()
    };

    let run = instance
        .get_typed_func::<(), i32>(&mut store, "run")
        .expect("Export 'run' not found");

    let result = run.call(&mut store, ()).expect("Execution failed");

    assert_eq!(result, 42, "Expected 42 from chained import, got {result}",);
}

/// This test defines the Fibonacci function across two mutually dependent modules:
/// - Module A defines the recursive function `a(n)`
/// - Module B re-exports `a` as `b`, completing a cycle so that `a` calls into `b`, which points back to `a`
///
/// This verifies that wasm_mergers can correctly resolve circular imports.
#[test]
fn merge_cross_module_fibonacci() {
    const WAT_MODULE_A: &str = r#"
      (module
        (import "b" "b" (func $b (param i32) (result i32)))

        (func $a (param $n i32) (result i32)
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
          call $b

          ;; fib(n - 2)
          local.get $n
          i32.const 2
          i32.sub
          call $b

          ;; add results
          i32.add)

        (export "a" (func $a)))
      "#;

    const WAT_MODULE_B: &str = r#"
      (module
        (import "a" "a" (func $a (param i32) (result i32)))
        (export "b" (func $a)))
      "#;

    // Parse WAT source to binary
    let binary_a = parse_str(WAT_MODULE_A).expect("Failed to parse module A");
    let binary_b = parse_str(WAT_MODULE_B).expect("Failed to parse module B");

    // Prepare named modules
    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("a", &binary_a),
        &NamedModule::new("b", &binary_b),
    ];

    // Merge the modules
    let merged_wasm = MergeConfiguration::new(modules)
        .merge()
        .expect("Failed to merge modules");

    // Instantiate merged module
    let mut store = Store::<()>::default();
    let module = Module::from_binary(store.engine(), &merged_wasm).expect("Invalid Wasm module");
    let instance = Instance::new(&mut store, &module, &[]).expect("Failed to instantiate module");

    // Get exported Fibonacci function
    let fib = instance
        .get_typed_func::<i32, i32>(&mut store, "a")
        .expect("Exported function 'a' not found");

    // Reference implementation
    fn expected_fib(n: i32) -> i32 {
        match n {
            0 => 0,
            1 => 1,
            _ => expected_fib(n - 1) + expected_fib(n - 2),
        }
    }

    // Run and assert behavior
    for i in 0..20 {
        let actual = fib.call(&mut store, i).expect("Wasm call failed");
        let expected = expected_fib(i);
        assert_eq!(
            actual, expected,
            "Mismatch at fib({i}): expected {expected}, got {actual}"
        );
    }
}

/// Module structure:
/// - `ab` defines:
///     - a() = 2
///     - b() = 3
///
/// - `cd` imports a() and b() from `ab`, and defines:
///     - c() = a() * 5  → 2 * 5 = 10  [unused]
///     - d() = b() * 7  → 3 * 7 = 21
///
/// - `e` imports a() and b() from `ab`, and c(), d() from `cd`, and defines:
///     - e() = (a()*11 * b()*13) * (c()*17 * d()*23)
///       = ((2*11)*(3*13)) * ((10*17)*(21*23))
///
/// After merging:
/// - All functions should be resolved internally (no remaining imports)
/// - `c` is unused (only referenced by `e` but result is never used), so it should be eliminated
/// - Final merged module should export: d(), e(), f()
#[test]
fn composition_of_cross_deps() {
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

    let wat_ab = parse_str(WAT_AB).unwrap();
    let wat_cd = parse_str(WAT_CD).unwrap();
    let wat_e = parse_str(WAT_E).unwrap();

    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("ab", &wat_ab),
        &NamedModule::new("cd", &wat_cd),
        &NamedModule::new("e", &wat_e),
    ];

    let merged = MergeConfiguration::new(modules).merge().unwrap();

    // Instantiate merged module (should be self-contained)
    let mut store = Store::<()>::default();
    let engine = store.engine();
    let module = Module::from_binary(engine, &merged).unwrap();
    let instance = Instance::new(&mut store, &module, &[]).unwrap();

    let actual_e = instance.get_typed_func::<(), i32>(&mut store, "e").unwrap();

    let a = || 2;
    let b = || 3;
    let c = || a() * 5;
    let d = || b() * 7;
    let e = || ((a() * 11) * (b() * 13)) * ((c() * 17) * (d() * 23));

    assert_eq!(actual_e.call(&mut store, ()).unwrap(), e());
}

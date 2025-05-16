use std::iter::once;

use itertools::Itertools;
use wasm_mergers::{MergeConfiguration, MergeOptions, NamedModule};
use wasmtime::*;
use wat::parse_str;

mod wasmtime_macros; // Bring macros in scope

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

        MergeConfiguration::new(modules, MergeOptions::default())
            .merge()
            .unwrap()
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

    let r_even = |v| v % 2 == 0;
    let r_odd = |v| !(r_even(v));

    // Behavioral assertion
    for merged_wasm in [lib_merged, manual_merged] {
        // Interpret even & odd
        let mut store = Store::<()>::default();
        let module = Module::from_binary(store.engine(), &merged_wasm).unwrap();
        let instance = Instance::new(&mut store, &module, &[]).unwrap();

        declare_fns_from_wasm! { instance, store,
           even [i32] [i32],
           odd [i32] [i32],
        };

        fn to_bool(v: i32) -> bool {
            assert!(v == 0 || v == 1);
            v == 1
        }

        for i in 0..1000 {
            assert_eq!(to_bool(wasm_call!(store, even, i)), r_even(i));
            assert_eq!(to_bool(wasm_call!(store, odd, i)), r_odd(i));
        }
    }
}

#[test]
fn test_earmark() {
    const NEEDLE: &[u8] = "wasm-mergers".as_bytes();
    const NEEDLE_LEN: usize = NEEDLE.len();
    const M: &str = "(module)";
    wasm_mergers::MergeConfiguration::new(
        &[
            &NamedModule::new("A", &wat::parse_str(M).unwrap()),
            &NamedModule::new("B", &wat::parse_str(M).unwrap()),
        ],
        MergeOptions::default(),
    )
    .merge()
    .unwrap()
    .windows(NEEDLE_LEN)
    .position(|w| NEEDLE == w)
    .unwrap();
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
            MergeConfiguration::new(&perm, MergeOptions::default())
                .merge()
                .unwrap()
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

    let merged = MergeConfiguration::new(modules, MergeOptions::default())
        .merge()
        .expect("Merge failed");

    // Instantiate & run merged module
    let mut store = Store::<()>::default();
    let module = Module::from_binary(store.engine(), &merged).unwrap();
    let instance = Instance::new(&mut store, &module, &[]).unwrap();

    declare_fns_from_wasm! {instance, store, run [] [i32]};
    let result = wasm_call!(store, run);

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

    // Parse WAT source to binary
    let binary_a = parse_str(WAT_MODULE_A).expect("Failed to parse module A");
    let binary_b = parse_str(WAT_MODULE_B).expect("Failed to parse module B");

    // Prepare named modules
    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("fib", &binary_a),
        &NamedModule::new("indirect_fib", &binary_b),
    ];

    // Merge the modules
    let merged_wasm = MergeConfiguration::new(modules, MergeOptions::default())
        .merge()
        .expect("Failed to merge modules");

    // Instantiate merged module
    let mut store = Store::<()>::default();
    let module = Module::from_binary(store.engine(), &merged_wasm).expect("Invalid Wasm module");
    let instance = Instance::new(&mut store, &module, &[]).expect("Failed to instantiate module");

    // Get exported Fibonacci function
    declare_fns_from_wasm! { instance, store, fib [i32] [i32] };

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
        let actual = wasm_call!(store, fib, i);
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

    let merged = MergeConfiguration::new(modules, MergeOptions::default())
        .merge()
        .unwrap();

    // Instantiate merged module (should be self-contained)
    let mut store = Store::<()>::default();
    let engine = store.engine();
    let module = Module::from_binary(engine, &merged).unwrap();
    let instance = Instance::new(&mut store, &module, &[]).unwrap();

    declare_fns_from_wasm! { instance, store, e [] [i32] };

    let rs_a = || 2;
    let rs_b = || 3;
    let rs_c = || rs_a() * 5;
    let rs_d = || rs_b() * 7;
    let rs_e = || ((rs_a() * 11) * (rs_b() * 13)) * ((rs_c() * 17) * (rs_d() * 23));

    assert_eq!(wasm_call!(store, e), rs_e());
}

#[test]
fn test_multi_memory() {
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

    let wasm_a = parse_str(gen_wat("a")).unwrap();
    let wasm_b = parse_str(gen_wat("b")).unwrap();

    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("A", &wasm_a),
        &NamedModule::new("B", &wasm_b),
    ];

    let merge_options = MergeOptions {
        rename_duplicate_exports: true,
    };

    let merged = MergeConfiguration::new(modules, merge_options)
        .merge()
        .unwrap();

    // Instantiate merged module (should be self-contained)
    let mut store = Store::<()>::default();
    let engine = store.engine();
    let module = Module::from_binary(engine, &merged).unwrap();
    let instance = Instance::new(&mut store, &module, &[]).unwrap();

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
}

/// Rust compilation with memory modules
#[test]
fn test_rust_compilation() {
    use rust_to_wasm_compiler::{Profile, RustToWasmCompiler, WasiSupport};

    const MANIFEST_SOURCE: &str = r#"
      package.name = "test_rust_compilation"
      package.edition = "2024"
      lib.crate-type = ["cdylib"]
      [workspace]
    "#;

    const LIB_SOURCE_EVEN: &str = r#"
      #[link(wasm_import_module = "odd")]
      unsafe extern "C" { fn unsafe_odd(v: i32) -> i32; }
      fn odd(v: i32) -> i32 { unsafe { unsafe_odd(v) } }

      #[unsafe(no_mangle)] pub extern "C" fn unsafe_even(v: i32) -> i32 { even(v) }
      #[unsafe(no_mangle)] pub extern "C" fn even(v: i32) -> i32 {
          if v == 0 { return 1; /* true (even) */ }
          else { return odd(v - 1); /* call odd function */ } }
    "#;

    const LIB_SOURCE_ODD: &str = r#"
      #[link(wasm_import_module = "even")]
      unsafe extern "C" { fn unsafe_even(v: i32) -> i32; }
      fn even(v: i32) -> i32 { unsafe { unsafe_even(v) } }

      #[unsafe(no_mangle)] pub extern "C" fn unsafe_odd(v: i32) -> i32 { odd(v) }
      #[unsafe(no_mangle)] pub extern "C" fn odd(v: i32) -> i32 {
          if v == 0 { return 0; /* false (not odd) */ }
          else { return even(v - 1); /* call even function */ } }
    "#;

    let compiler = RustToWasmCompiler::new().unwrap();
    let compile = |source| {
        compiler
            .compile_source(
                WasiSupport::Disabled,
                MANIFEST_SOURCE,
                source,
                Profile::Release,
            )
            .unwrap()
    };
    let wasm_even = compile(LIB_SOURCE_EVEN);
    let wasm_odd = compile(LIB_SOURCE_ODD);

    // Merge & test merged

    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("even", &wasm_even),
        &NamedModule::new("odd", &wasm_odd),
    ];

    let options = MergeOptions {
        rename_duplicate_exports: true,
    };

    let lib_merged = MergeConfiguration::new(modules, options).merge().unwrap();

    // Structural assertion
    {
        let simply_appended_len = (wasm_even.len() + wasm_odd.len()) as f64;
        let lib_merged_len = lib_merged.len() as f64;
        let ratio = simply_appended_len / lib_merged_len;
        const RATIO_ALLOWED_DELTA: f64 = 3.00; // 300% difference
        assert!(
            (1.0 - RATIO_ALLOWED_DELTA..=1.0 + RATIO_ALLOWED_DELTA).contains(&ratio),
            "Lengths differ by more than 50%: manual = {simply_appended_len}, lib = {lib_merged_len}",
        );
    }

    let r_even = |v| v % 2 == 0;
    let r_odd = |v| !(r_even(v));

    // Behavioral assertion
    for merged_wasm in [lib_merged] {
        // Interpret even & odd
        let mut store = Store::<()>::default();
        let module = Module::from_binary(store.engine(), &merged_wasm).unwrap();
        let instance = Instance::new(&mut store, &module, &[]).unwrap();

        // Fetch `even` and `odd` export
        declare_fns_from_wasm! {
          instance, store,
          even [i32] [i32],
          odd [i32] [i32],
        }

        let to_bool = |v| {
            assert!(v == 0 || v == 1);
            v == 1
        };

        for i in 0..1000 {
            assert_eq!(to_bool(wasm_call!(store, even, i)), r_even(i));
            assert_eq!(to_bool(wasm_call!(store, odd, i)), r_odd(i));
        }
    }
}

/// Rust compilation with memory modules
#[test]
fn test_rust_compilation_tables() {
    use rust_to_wasm_compiler::{Profile, RustToWasmCompiler, WasiSupport};

    const MANIFEST_SOURCE: &str = r#"
      package.name = "test_rust_compilation"
      package.edition = "2024"
      lib.crate-type = ["cdylib"]
      [workspace]
    "#;

    const LIB_SOURCE_EVEN: &str = r#"
    static mut EVEN_FN_PTR: Option<extern "C" fn(i32) -> i32> = None;

    #[link(wasm_import_module = "odd")]
    unsafe extern "C" { fn unsafe_odd(v: i32) -> i32; }
    fn odd(v: i32) -> i32 { unsafe { unsafe_odd(v) } }

    #[unsafe(no_mangle)] pub extern "C" fn unsafe_even(v: i32) -> i32 { even(v) }
    #[unsafe(no_mangle)] pub extern "C" fn even(v: i32) -> i32 {
        if v == 0 { return 1; /* true (even) */ }
        else { return odd(v - 1); /* call odd function */ } }

    #[unsafe(no_mangle)] pub extern "C" fn install_even() {
      unsafe { EVEN_FN_PTR = Some(unsafe_even); }
    }
  "#;

    const LIB_SOURCE_ODD: &str = r#"
    #[link(wasm_import_module = "even")]
    unsafe extern "C" { fn unsafe_even(v: i32) -> i32; }
    fn even(v: i32) -> i32 { unsafe { unsafe_even(v) } }

    #[unsafe(no_mangle)] pub extern "C" fn unsafe_odd(v: i32) -> i32 { odd(v) }
    #[unsafe(no_mangle)] pub extern "C" fn odd(v: i32) -> i32 {
        if v == 0 { return 0; /* false (not odd) */ }
        else { return even(v - 1); /* call even function */ } }
  "#;

    let compiler = RustToWasmCompiler::new().unwrap();
    let compile = |source| {
        compiler
            .compile_source(WasiSupport::Disabled, MANIFEST_SOURCE, source, Profile::Dev)
            .unwrap()
    };
    let wasm_even = compile(LIB_SOURCE_EVEN);
    let wasm_odd = compile(LIB_SOURCE_ODD);

    // Merge & test merged

    let modules: &[&NamedModule<'_, &[u8]>] = &[
        &NamedModule::new("even", &wasm_even),
        &NamedModule::new("odd", &wasm_odd),
    ];

    let options = MergeOptions {
        rename_duplicate_exports: true,
    };

    let lib_merged = MergeConfiguration::new(modules, options).merge().unwrap();

    // Structural assertion not included.
    // FIXME: Since debug support is not enabled, we will not assert on the sizes

    let r_even = |v| v % 2 == 0;
    let r_odd = |v| !(r_even(v));

    // Behavioral assertion
    for merged_wasm in [lib_merged] {
        // Interpret even & odd
        let mut store = Store::<()>::default();
        let module = Module::from_binary(store.engine(), &merged_wasm).unwrap();
        let instance = Instance::new(&mut store, &module, &[]).unwrap();

        declare_fns_from_wasm! {
          instance, store,
          even [i32] [i32],
          odd [i32] [i32],
          install_even [] [],
        }

        wasm_call!(store, install_even);

        let to_bool = |v| {
            assert!(v == 0 || v == 1);
            v == 1
        };

        for i in 0..2 {
            assert_eq!(to_bool(wasm_call!(store, even, i)), r_even(i));
            assert_eq!(to_bool(wasm_call!(store, odd, i)), r_odd(i));
        }
    }
}

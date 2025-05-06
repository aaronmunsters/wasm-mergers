use wasm_mergers::{MergeConfiguration, NamedModule};
use wasmtime::*;
use wat::parse_str;

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

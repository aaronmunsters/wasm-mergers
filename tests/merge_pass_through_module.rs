use wasm_mergers::{MergeConfiguration, NamedModule};
use wasmtime::*;
use wat::parse_str;

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

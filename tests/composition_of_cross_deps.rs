use wasm_mergers::{MergeConfiguration, NamedModule};
use wasmtime::*;
use wat::parse_str;

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

use wasm_mergers::NamedModule;
use wasm_mergers::merge_options::DEFAULT_RENAMER;
use wasm_mergers::merge_options::{ClashingExports, MergeOptions};

use arbitrary::Unstructured;
use rand_chacha::rand_core::{Rng, SeedableRng};
use rayon::prelude::*;
use wasm_smith::{Config as WasmSmithConfig, Module as WasmSmithModule};
use wasmtime::*;

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
const MAX_PRGS: usize = 2_usize.pow(10);
const WINDOW_NAMES: &[&str] = &["a", "b", "c", "d"];

fn test_config() -> WasmSmithConfig {
    WasmSmithConfig {
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
    }
}

fn get_expected_outcomes() -> Vec<ExpectedModuleOutcomes> {
    (0..MAX_SEED)
        .into_par_iter()
        .filter_map(|seed| {
            let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let mut random_sequence = [0_u8; MAX_PRGS];
            rng.fill_bytes(&mut random_sequence);
            let mut random = Unstructured::new(&random_sequence);
            let mut module = WasmSmithModule::new(test_config(), &mut random).unwrap();
            module.ensure_termination(10_000).unwrap();
            let module_bytes = module.to_bytes();

            // Instantiate merged module (should be self-contained)
            let config = Config::new();
            let engine = Engine::new(&config).unwrap();
            let mut store = Store::<()>::new(&engine, ());
            let module = Module::from_binary(&engine, &module_bytes).unwrap();
            let Ok(instance) = Instance::new(&mut store, &module, &[]) else {
                // It could still be that instantiation fails from the WasmSmith generated module
                return None;
            };

            let mut random_val = |p| match p {
                ValType::I32 => Some(Val::I32(rng.next_u32().cast_signed())),
                ValType::I64 => Some(Val::I64(rng.next_u64().cast_signed())),
                ValType::F32 => Some(Val::F32(rng.next_u32())),
                ValType::F64 => Some(Val::F64(rng.next_u64())),
                _ => None,
            };

            let call_results: Vec<PreMergeOutcome> = module
                .exports()
                .filter_map(|export| match export.ty() {
                    ExternType::Func(f_ty) => {
                        let args: Vec<_> =
                            f_ty.params().map(&mut random_val).collect::<Option<_>>()?;
                        let mut results: Vec<_> =
                            f_ty.results().map(&mut random_val).collect::<Option<_>>()?;
                        instance
                            .get_func(&mut store, export.name())
                            .unwrap()
                            .call(&mut store, &args, &mut results)
                            .ok()
                            .map(|()| {
                                // If the call succeeded, we *finally* have a pre-merge assertion
                                PreMergeOutcome {
                                    args,
                                    results,
                                    function_name: export.name().to_string(),
                                }
                            })
                    }
                    _ => None,
                })
                .collect();

            Some(ExpectedModuleOutcomes {
                module: module_bytes,
                expected_outcomes: call_results,
            })
        })
        .collect()
}

#[test]
fn test_smithed_modules() {
    let window_width: usize = WINDOW_NAMES.len();
    get_expected_outcomes()
        .windows(window_width)
        .for_each(|window| {
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
                wasm_mergers::MergeConfiguration::new(modules, merge_options);
            let merged = merge_configuration.merge();

            // Failing to parse is something related to the crates `wasm-smith` <~> `walrus`
            if let Err(wasm_mergers::error::Error::Parse(_)) = merged {
                return;
            }

            // Unwrap the module, asserting it exists
            let merged = merged.unwrap();

            // Instantiate merged module (should be self-contained)
            let config = Config::new();
            let engine = Engine::new(&config).unwrap();
            let mut store = Store::<()>::new(&engine, ());
            let module = Module::from_binary(&engine, &merged).unwrap();
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
                        if let Some(func) = instance.get_func(&mut store, &assertion.function_name)
                        {
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

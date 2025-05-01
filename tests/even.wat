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

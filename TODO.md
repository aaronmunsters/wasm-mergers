
// Another way would be to 'split' modules eg. the benchmark into multiple
// variants ...

/*
CASE 1: 🟠
(mod A
    (def 0 ...)
    (export 0 as "a" ...))

(mod B
    (import "a" as $a ...)
    (export $a as "z" ...))

(mod C
    (import "z" as $z ...)
    (call $z))
*/

/*
CASE 2: 🔴
(mod A
    (import "z" as $z ...)
    (def 0 ...)
    (export 0 as "a" ...)
    (call $z))

(mod B
    (import "a" as $a ...)
    (export $a as "z" ...))
*/

/*
CASE 3: 🟣
        (mod
            (import "a" as 0)
            (import "b" as 1)
            (import "c" as 2)
            (def 3 as "d" ...call-0...)
            (def 4 as "e" ...call-1...))

        (mod
            (def 0 as "a" ...)
            (def 1 as "a" ...)
            (export "a" as 0)
            (export "b" as 1))

        (mod
            (import "b" as 0)
            (def 1 as "f" ...call-0...))

        ==> Merged:

        (mod
            (def 0 as "a" ...)
            (def 1 as "b" ...)
            (import "c" as 2)
            (def 3 as "d" ...call-0...)
            (def 4 as "e ...call-1...)
            (def 5 as "f" ...call-1...))
        */

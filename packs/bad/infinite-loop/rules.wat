;; A rules module that answers "info" and then spins forever.
(module
  (memory (export "memory") 1)
  (data (i32.const 64) "{\"ok\":{\"min_seats\":2,\"max_seats\":2,\"api\":1}}")
  (func (export "hostable_abi_version") (result i32) (i32.const 1))
  (func (export "hostable_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "hostable_free") (param i32 i32))
  (func (export "hostable_call") (param $p i32) (param $n i32) (result i64)
    ;; The 10th byte of {"call":"info"} is the i of info.
    (if (i32.eq (i32.load8_u (i32.add (local.get $p) (i32.const 9))) (i32.const 105))
      (then (return (i64.or (i64.shl (i64.const 64) (i64.const 32)) (i64.const 44)))))
    (loop $spin (br $spin)) (unreachable)))

  emit_str "minute,requests,errors,p50_ms,total\n"
loop:
  get 0
  push 60
  mul
  push 1700000000
  add
  emit_dec
  emit_str ","
  get 0
  push 37
  mul
  push 251
  mod
  push 1000
  add
  dup
  get 1
  add
  set 1
  emit_dec
  emit_str ","
  get 0
  dup
  mul
  push 7
  mod
  emit_dec
  emit_str ","
  get 0
  push 60
  mod
  push 20
  add
  emit_dec
  emit_str ","
  get 1
  emit_dec
  emit_str "\n"
  get 0
  push 1
  add
  dup
  set 0
  push 4000
  lt
  jnz loop
  halt

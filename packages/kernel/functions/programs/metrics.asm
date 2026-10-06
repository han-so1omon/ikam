; metrics.csv: 4,000 rows of computed columns and a running total
  emit_str "minute,requests,errors,p50_ms,total\n"
  push 0
  set 0            ; i
  push 0
  set 1            ; total
row:
  get 0
  push 37
  mul
  push 251
  mod
  push 1000
  add
  set 2            ; req = 1000 + (37 i) % 251
  get 1
  get 2
  add
  set 1            ; total += req
  get 0
  push 60
  mul
  push 1700000000
  add
  emit_dec
  emit_str ","
  get 2
  emit_dec
  emit_str ","
  get 0
  get 0
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
  jnz row
  halt

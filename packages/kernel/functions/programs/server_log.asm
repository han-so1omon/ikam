; server.log: 6,000 lines; status, path and latency from a 64-bit LCG
  push 2685821657736338717
  set 9            ; LCG state
  push 0
  set 0            ; i
line:
  get 9
  push 6364136223846793005
  mul
  push 1442695040888963407
  add
  dup
  set 9
  push 33
  shr
  push 6
  mod
  set 3            ; status index (drawn first)
  emit_str "2026-10-06T12:"
  get 0
  push 60
  div
  push 60
  mod
  emit_pad 2 '0'
  emit_str ":"
  get 0
  push 60
  mod
  emit_pad 2 '0'
  emit_str "Z req="
  get 0
  push 100000
  add
  emit_pad 6 '0'
  emit_str " "
  get 9
  push 6364136223846793005
  mul
  push 1442695040888963407
  add
  dup
  set 9
  push 33
  shr
  push 5
  mod
  emit_sel "/api/users" "/api/orders" "/health" "/static/app.js" "/api/search"
  emit_str " "
  get 3
  emit_sel "200" "200" "200" "304" "404" "500"
  emit_str " "
  get 9
  push 6364136223846793005
  mul
  push 1442695040888963407
  add
  dup
  set 9
  push 33
  shr
  push 200
  mod
  push 5
  add
  emit_dec
  emit_str "ms\n"
  get 0
  push 1
  add
  dup
  set 0
  push 6000
  lt
  jnz line
  halt

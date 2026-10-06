  push 1
  set 0
outer:
  push 1
  set 1
inner:
  get 0
  get 1
  mul
  emit_pad 5 ' '
  get 1
  push 1
  add
  dup
  set 1
  push 100
  lt
  jnz inner
  emit_str "\n"
  get 0
  push 1
  add
  dup
  set 0
  push 100
  lt
  jnz outer
  halt

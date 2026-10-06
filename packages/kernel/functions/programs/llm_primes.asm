  push 2
  set 0
loop:
  get 0
  load
  jnz next
  get 0
  emit_dec
  emit_str "\n"
  get 1
  push 1
  add
  dup
  set 1
  push 20000
  eq
  jnz end
  get 0
  dup
  mul
  set 2
mark:
  get 2
  push 224738
  lt
  jz next
  push 1
  get 2
  store
  get 2
  get 0
  add
  set 2
  jmp mark
next:
  get 0
  push 1
  add
  set 0
  jmp loop
end:
  halt

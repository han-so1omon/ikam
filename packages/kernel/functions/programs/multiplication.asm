; multiplication.txt: 99 x 99 table, each product right-aligned in 5 columns
  push 1
  set 0          ; a = 1
row:
  push 1
  set 1          ; b = 1
cell:
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
  jnz cell
  emit_str "\n"
  get 0
  push 1
  add
  dup
  set 0
  push 100
  lt
  jnz row
  halt

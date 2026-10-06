; mandelbrot.pgm: 320 x 240, fixed point with 12 fractional bits
  emit_str "P5\n320 240\n255\n"
  push 0
  set 1            ; y
yloop:
  push 0
  set 0            ; x
xloop:
  get 0
  push 13107
  mul
  push 320
  div
  push -9011
  add
  set 2            ; cr
  get 1
  push 9830
  mul
  push 240
  div
  push -4915
  add
  set 3            ; ci
  push 0
  set 4            ; zr
  push 0
  set 5            ; zi
  push 0
  set 6            ; n
iter:
  get 6
  push 255
  lt
  jz done
  get 4
  get 4
  mul
  get 5
  get 5
  mul
  add
  push 67108864
  swap
  lt               ; 4 << 24 < |z|^2 ?
  jnz done
  get 4
  get 4
  mul
  get 5
  get 5
  mul
  sub
  push 12
  sar
  get 2
  add
  set 7            ; new zr
  push 2
  get 4
  mul
  get 5
  mul
  push 12
  sar
  get 3
  add
  set 5            ; new zi
  get 7
  set 4
  get 6
  push 1
  add
  set 6
  jmp iter
done:
  get 6
  emit_byte
  get 0
  push 1
  add
  dup
  set 0
  push 320
  lt
  jnz xloop
  get 1
  push 1
  add
  dup
  set 1
  push 240
  lt
  jnz yloop
  halt

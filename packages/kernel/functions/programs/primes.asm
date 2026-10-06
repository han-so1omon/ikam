; primes.txt: the first 20,000 primes (sieve of Eratosthenes below 224,738)
  push 2
  set 0            ; i
sieve:
  get 0
  get 0
  mul
  push 224738
  lt
  jz print         ; while i * i < N
  get 0
  load
  jnz skip         ; i is composite
  get 0
  get 0
  mul
  set 1            ; j = i * i
mark:
  get 1
  push 224738
  lt
  jz skip
  push 1
  get 1
  store            ; composite[j] = 1
  get 1
  get 0
  add
  set 1
  jmp mark
skip:
  get 0
  push 1
  add
  set 0
  jmp sieve
print:
  push 2
  set 0            ; n
  push 0
  set 2            ; count
next:
  get 2
  push 20000
  lt
  jz end
  get 0
  load
  jnz composite
  get 0
  emit_dec
  emit_str "\n"
  get 2
  push 1
  add
  set 2
composite:
  get 0
  push 1
  add
  set 0
  jmp next
end:
  halt

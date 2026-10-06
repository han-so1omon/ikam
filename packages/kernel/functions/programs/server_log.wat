(module
  (memory (export "memory") 1)
  (data (i32.const 1024) "\32\30\32\36\2d\31\30\2d\30\36\54\31\32\3a\3a\5a\20\72\65\71\3d\20\2f\61\70\69\2f\75\73\65\72\73\2f\61\70\69\2f\6f\72\64\65\72\73\2f\68\65\61\6c\74\68\2f\73\74\61\74\69\63\2f\61\70\70\2e\6a\73\2f\61\70\69\2f\73\65\61\72\63\68\32\30\30\33\30\34\34\30\34\35\30\30\6d\73\0a")
  (global $heap (mut i32) (i32.const 65536))
  (global $out (mut i32) (i32.const 0))
  (global $out0 (mut i32) (i32.const 0))
  (func $ensure (param $end i32)
    (local $have i32)
    (local.set $have (i32.shl (memory.size) (i32.const 16)))
    (if (i32.gt_u (local.get $end) (local.get $have))
      (then (if (i32.lt_s (memory.grow (i32.shr_u (i32.add (i32.sub (local.get $end) (local.get $have)) (i32.const 65535)) (i32.const 16))) (i32.const 0))
        (then unreachable)))))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (local.get $p) (local.get $n)))
    (call $ensure (global.get $heap))
    (local.get $p))
  (func $emit_byte (param $v i64)
    (if (i32.ge_u (global.get $out) (i32.const 33554432)) (then unreachable))
    (call $ensure (i32.add (global.get $out) (i32.const 1)))
    (i64.store8 (global.get $out) (local.get $v))
    (global.set $out (i32.add (global.get $out) (i32.const 1))))
  (func $emit_bytes (param $p i32) (param $n i32)
    (if (i32.ge_u (i32.add (global.get $out) (local.get $n)) (i32.const 33554432)) (then unreachable))
    (call $ensure (i32.add (global.get $out) (local.get $n)))
    (memory.copy (global.get $out) (local.get $p) (local.get $n))
    (global.set $out (i32.add (global.get $out) (local.get $n))))
  (func $emit_pad (param $v i64) (param $w i32) (param $f i32)
    (local $u i64) (local $i i32) (local $len i32)
    ;; digits of |v| into 512..532, right to left
    (local.set $u (select (i64.sub (i64.const 0) (local.get $v)) (local.get $v) (i64.lt_s (local.get $v) (i64.const 0))))
    (local.set $i (i32.const 532))
    (loop $d
      (local.set $i (i32.sub (local.get $i) (i32.const 1)))
      (i64.store8 (local.get $i) (i64.add (i64.const 48) (i64.rem_u (local.get $u) (i64.const 10))))
      (local.set $u (i64.div_u (local.get $u) (i64.const 10)))
      (br_if $d (i64.ne (local.get $u) (i64.const 0))))
    (local.set $len (i32.add (i32.sub (i32.const 532) (local.get $i)) (i64.lt_s (local.get $v) (i64.const 0))))
    (block $done (loop $p
      (br_if $done (i32.ge_s (local.get $len) (local.get $w)))
      (call $emit_byte (i64.extend_i32_u (local.get $f)))
      (local.set $w (i32.sub (local.get $w) (i32.const 1)))
      (br $p)))
    (if (i64.lt_s (local.get $v) (i64.const 0)) (then (call $emit_byte (i64.const 45))))
    (call $emit_bytes (local.get $i) (i32.sub (i32.const 532) (local.get $i))))
  (func $addr (param $i i64) (result i32)
    (if (i64.ge_u (local.get $i) (i64.const 4194304)) (then unreachable))
    (i32.add (i32.const 33554432) (i32.wrap_i64 (i64.shl (local.get $i) (i64.const 3)))))
  (func $load (param $i i64) (result i64)
    (local $a i32)
    (local.set $a (call $addr (local.get $i)))
    (if (result i64) (i32.lt_u (local.get $a) (i32.shl (memory.size) (i32.const 16)))
      (then (i64.load (local.get $a))) (else (i64.const 0))))
  (func $store (param $v i64) (param $i i64)
    (local $a i32)
    (local.set $a (call $addr (local.get $i)))
    (call $ensure (i32.add (local.get $a) (i32.const 8)))
    (i64.store (local.get $a) (local.get $v)))
  (func (export "run") (param $ptr i32) (param $len i32) (result i64)
    (local $pc i32) (local $t0 i64) (local $t1 i64) (local $r0 i64) (local $r1 i64) (local $r2 i64) (local $r3 i64) (local $r4 i64) (local $r5 i64) (local $r6 i64) (local $r7 i64) (local $r8 i64) (local $r9 i64) (local $r10 i64) (local $r11 i64) (local $r12 i64) (local $r13 i64) (local $r14 i64) (local $r15 i64) (local $r16 i64) (local $r17 i64) (local $r18 i64) (local $r19 i64) (local $r20 i64) (local $r21 i64) (local $r22 i64) (local $r23 i64) (local $r24 i64) (local $r25 i64) (local $r26 i64) (local $r27 i64) (local $r28 i64) (local $r29 i64) (local $r30 i64) (local $r31 i64) (local $r32 i64) (local $r33 i64) (local $r34 i64) (local $r35 i64) (local $r36 i64) (local $r37 i64) (local $r38 i64) (local $r39 i64) (local $r40 i64) (local $r41 i64) (local $r42 i64) (local $r43 i64) (local $r44 i64) (local $r45 i64) (local $r46 i64) (local $r47 i64) (local $r48 i64) (local $r49 i64) (local $r50 i64) (local $r51 i64) (local $r52 i64) (local $r53 i64) (local $r54 i64) (local $r55 i64) (local $r56 i64) (local $r57 i64) (local $r58 i64) (local $r59 i64) (local $r60 i64) (local $r61 i64) (local $r62 i64) (local $r63 i64) (local $r64 i64) (local $r65 i64) (local $r66 i64) (local $r67 i64) (local $r68 i64) (local $r69 i64) (local $r70 i64) (local $r71 i64) (local $r72 i64) (local $r73 i64) (local $r74 i64) (local $r75 i64) (local $r76 i64) (local $r77 i64) (local $r78 i64) (local $r79 i64) (local $r80 i64) (local $r81 i64) (local $r82 i64) (local $r83 i64) (local $r84 i64) (local $r85 i64) (local $r86 i64) (local $r87 i64) (local $r88 i64) (local $r89 i64) (local $r90 i64) (local $r91 i64) (local $r92 i64) (local $r93 i64) (local $r94 i64) (local $r95 i64) (local $r96 i64) (local $r97 i64) (local $r98 i64) (local $r99 i64) (local $r100 i64) (local $r101 i64) (local $r102 i64) (local $r103 i64) (local $r104 i64) (local $r105 i64) (local $r106 i64) (local $r107 i64) (local $r108 i64) (local $r109 i64) (local $r110 i64) (local $r111 i64) (local $r112 i64) (local $r113 i64) (local $r114 i64) (local $r115 i64) (local $r116 i64) (local $r117 i64) (local $r118 i64) (local $r119 i64) (local $r120 i64) (local $r121 i64) (local $r122 i64) (local $r123 i64) (local $r124 i64) (local $r125 i64) (local $r126 i64) (local $r127 i64) (local $r128 i64) (local $r129 i64) (local $r130 i64) (local $r131 i64) (local $r132 i64) (local $r133 i64) (local $r134 i64) (local $r135 i64) (local $r136 i64) (local $r137 i64) (local $r138 i64) (local $r139 i64) (local $r140 i64) (local $r141 i64) (local $r142 i64) (local $r143 i64) (local $r144 i64) (local $r145 i64) (local $r146 i64) (local $r147 i64) (local $r148 i64) (local $r149 i64) (local $r150 i64) (local $r151 i64) (local $r152 i64) (local $r153 i64) (local $r154 i64) (local $r155 i64) (local $r156 i64) (local $r157 i64) (local $r158 i64) (local $r159 i64) (local $r160 i64) (local $r161 i64) (local $r162 i64) (local $r163 i64) (local $r164 i64) (local $r165 i64) (local $r166 i64) (local $r167 i64) (local $r168 i64) (local $r169 i64) (local $r170 i64) (local $r171 i64) (local $r172 i64) (local $r173 i64) (local $r174 i64) (local $r175 i64) (local $r176 i64) (local $r177 i64) (local $r178 i64) (local $r179 i64) (local $r180 i64) (local $r181 i64) (local $r182 i64) (local $r183 i64) (local $r184 i64) (local $r185 i64) (local $r186 i64) (local $r187 i64) (local $r188 i64) (local $r189 i64) (local $r190 i64) (local $r191 i64) (local $r192 i64) (local $r193 i64) (local $r194 i64) (local $r195 i64) (local $r196 i64) (local $r197 i64) (local $r198 i64) (local $r199 i64) (local $r200 i64) (local $r201 i64) (local $r202 i64) (local $r203 i64) (local $r204 i64) (local $r205 i64) (local $r206 i64) (local $r207 i64) (local $r208 i64) (local $r209 i64) (local $r210 i64) (local $r211 i64) (local $r212 i64) (local $r213 i64) (local $r214 i64) (local $r215 i64) (local $r216 i64) (local $r217 i64) (local $r218 i64) (local $r219 i64) (local $r220 i64) (local $r221 i64) (local $r222 i64) (local $r223 i64) (local $r224 i64) (local $r225 i64) (local $r226 i64) (local $r227 i64) (local $r228 i64) (local $r229 i64) (local $r230 i64) (local $r231 i64) (local $r232 i64) (local $r233 i64) (local $r234 i64) (local $r235 i64) (local $r236 i64) (local $r237 i64) (local $r238 i64) (local $r239 i64) (local $r240 i64) (local $r241 i64) (local $r242 i64) (local $r243 i64) (local $r244 i64) (local $r245 i64) (local $r246 i64) (local $r247 i64) (local $r248 i64) (local $r249 i64) (local $r250 i64) (local $r251 i64) (local $r252 i64) (local $r253 i64) (local $r254 i64) (local $r255 i64)
    (global.set $out0 (global.get $heap))
    (global.set $out (global.get $heap))
    block $exit
    loop $dispatch
    block $b2
    block $b1
    block $b0
    local.get $pc
    br_table $b0 $b1 $b2 $b0
    end ;; dispatch table
    i64.const 2685821657736338717
    local.set $r9
    i64.const 0
    local.set $r0
    end ;; block 1
    local.get $r9
    i64.const 6364136223846793005
    i64.mul
    i64.const 1442695040888963407
    i64.add
    local.tee $t0
    local.get $t0
    local.set $r9
    i64.const 33
    i64.shr_u
    i64.const 6
    i64.rem_s
    local.set $r3
    i32.const 1024
    i32.const 14
    call $emit_bytes
    local.get $r0
    i64.const 60
    i64.div_s
    i64.const 60
    i64.rem_s
    i32.const 2
    i32.const 48
    call $emit_pad
    i32.const 1038
    i32.const 1
    call $emit_bytes
    local.get $r0
    i64.const 60
    i64.rem_s
    i32.const 2
    i32.const 48
    call $emit_pad
    i32.const 1039
    i32.const 6
    call $emit_bytes
    local.get $r0
    i64.const 100000
    i64.add
    i32.const 6
    i32.const 48
    call $emit_pad
    i32.const 1045
    i32.const 1
    call $emit_bytes
    local.get $r9
    i64.const 6364136223846793005
    i64.mul
    i64.const 1442695040888963407
    i64.add
    local.tee $t0
    local.get $t0
    local.set $r9
    i64.const 33
    i64.shr_u
    i64.const 5
    i64.rem_s
    local.set $t0
    local.get $t0
    i64.const 0
    i64.eq
    if
    i32.const 1046
    i32.const 10
    call $emit_bytes
    end
    local.get $t0
    i64.const 1
    i64.eq
    if
    i32.const 1056
    i32.const 11
    call $emit_bytes
    end
    local.get $t0
    i64.const 2
    i64.eq
    if
    i32.const 1067
    i32.const 7
    call $emit_bytes
    end
    local.get $t0
    i64.const 3
    i64.eq
    if
    i32.const 1074
    i32.const 14
    call $emit_bytes
    end
    local.get $t0
    i64.const 4
    i64.eq
    if
    i32.const 1088
    i32.const 11
    call $emit_bytes
    end
    i32.const 1045
    i32.const 1
    call $emit_bytes
    local.get $r3
    local.set $t0
    local.get $t0
    i64.const 0
    i64.eq
    if
    i32.const 1099
    i32.const 3
    call $emit_bytes
    end
    local.get $t0
    i64.const 1
    i64.eq
    if
    i32.const 1099
    i32.const 3
    call $emit_bytes
    end
    local.get $t0
    i64.const 2
    i64.eq
    if
    i32.const 1099
    i32.const 3
    call $emit_bytes
    end
    local.get $t0
    i64.const 3
    i64.eq
    if
    i32.const 1102
    i32.const 3
    call $emit_bytes
    end
    local.get $t0
    i64.const 4
    i64.eq
    if
    i32.const 1105
    i32.const 3
    call $emit_bytes
    end
    local.get $t0
    i64.const 5
    i64.eq
    if
    i32.const 1108
    i32.const 3
    call $emit_bytes
    end
    i32.const 1045
    i32.const 1
    call $emit_bytes
    local.get $r9
    i64.const 6364136223846793005
    i64.mul
    i64.const 1442695040888963407
    i64.add
    local.tee $t0
    local.get $t0
    local.set $r9
    i64.const 33
    i64.shr_u
    i64.const 200
    i64.rem_s
    i64.const 5
    i64.add
    i32.const 0
    i32.const 32
    call $emit_pad
    i32.const 1111
    i32.const 3
    call $emit_bytes
    local.get $r0
    i64.const 1
    i64.add
    local.tee $t0
    local.get $t0
    local.set $r0
    i64.const 6000
    i64.lt_s
    i64.extend_i32_u
    i64.const 0
i64.ne
    if
    i32.const 1
    local.set $pc
    br $dispatch
    end
    end ;; block 2
    br $exit
    br $exit
    end
    end
    (i64.or (i64.shl (i64.extend_i32_u (global.get $out0)) (i64.const 32))
            (i64.extend_i32_u (i32.sub (global.get $out) (global.get $out0))))))

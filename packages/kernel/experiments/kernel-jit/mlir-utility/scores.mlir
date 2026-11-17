// E342: E341's two kernels as plain linalg on tensors, for IREE to compile
// for the CPU and for an Apple GPU (Metal) from this one file.
func.func @scores(%a: tensor<31600x256xf32>, %b: tensor<256x316xf32>) -> tensor<31600x316xf32> {
  %z = arith.constant 0.0 : f32
  %e = tensor.empty() : tensor<31600x316xf32>
  %f = linalg.fill ins(%z : f32) outs(%e : tensor<31600x316xf32>) -> tensor<31600x316xf32>
  %r = linalg.matmul ins(%a, %b : tensor<31600x256xf32>, tensor<256x316xf32>) outs(%f : tensor<31600x316xf32>) -> tensor<31600x316xf32>
  return %r : tensor<31600x316xf32>
}
func.func @scan(%r: tensor<200000x256xf32>, %q: tensor<256xf32>) -> tensor<200000xf32> {
  %z = arith.constant 0.0 : f32
  %e = tensor.empty() : tensor<200000xf32>
  %f = linalg.fill ins(%z : f32) outs(%e : tensor<200000xf32>) -> tensor<200000xf32>
  %s = linalg.matvec ins(%r, %q : tensor<200000x256xf32>, tensor<256xf32>) outs(%f : tensor<200000xf32>) -> tensor<200000xf32>
  return %s : tensor<200000xf32>
}

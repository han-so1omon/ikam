func.func @score(%a: vector<16xf32>, %b: vector<16xf32>) -> f32 {
  %s = "ikam.dot"(%a, %b) : (vector<16xf32>, vector<16xf32>) -> f32
  return %s : f32
}

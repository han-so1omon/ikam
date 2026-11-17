func.func @score(%a: vector<16xf32>, %b: vector<16xf32>) -> f64 {
  %s = "ikam.dot"(%a, %b) : (vector<16xf32>, vector<16xf32>) -> f64
  return %s : f64
}

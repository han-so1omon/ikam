const axios = require('axios');

async function main() {
  const graph_id = encodeURIComponent("s-local-retail-v01#1");
  try {
    const res = await axios.get(`http://localhost:8040/benchmarks/graph/nodes?graph_id=${graph_id}`);
    console.log("Nodes count:", res.data?.length);
  } catch(e) {
    console.log("Error:", e.message);
  }
}
main();

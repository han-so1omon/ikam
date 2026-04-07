const { getDebugStream } = require('./src/lib/api');

async function main() {
  const run_id = "run-d7bc0917442b449fadccbe2f0e3eedd4";
  const pipeline_id = "compression-rerender/v1";
  const pipeline_run_id = "run-d7bc0917442b449fadccbe2f0e3eedd4";
  try {
    const streamInfo = await getDebugStream(run_id, pipeline_id, pipeline_run_id);
    console.log("streamInfo:", streamInfo);
  } catch (err) {
    console.log("error:", err.message);
  }
}
main();

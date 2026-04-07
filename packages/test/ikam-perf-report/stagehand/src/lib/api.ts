import axios from 'axios';

const API_BASE = process.env.IKAM_STAGEHAND_API_URL || 'http://localhost:8040';

export async function getDebugStream(runId: string, pipelineId: string, pipelineRunId: string) {
  const response = await axios.get(`${API_BASE}/benchmarks/runs/${runId}/debug-stream`, {
    params: { pipeline_id: pipelineId, pipeline_run_id: pipelineRunId }
  });
  return typeof response.data === 'string' ? JSON.parse(response.data) : response.data;
}

export async function getRuns() {
  const response = await axios.get(`${API_BASE}/benchmarks/runs`);
  return typeof response.data === 'string' ? JSON.parse(response.data) : response.data;
}

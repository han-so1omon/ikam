from typing import Any, Dict
from llama_index.core.node_parser import SentenceSplitter
from llama_index.core.schema import Document
import uuid

def run(payload: Dict[str, Any], context: Dict[str, Any]) -> Any:
    """
    Omni-Parser operation (lift_fragments).
    Consumes parsed Document data, runs LlamaIndex chunking, and yields a subgraph
    of IKAM IR fragments representing the chunks and their structural relations.
    """
    fragment = payload.get("fragment", {})
    params = payload.get("params", {})
    
    documents_data = params.get("documents", [])
    if not documents_data:
        # fallback to checking fragment
        if isinstance(fragment, dict) and "documents" in fragment:
            documents_data = fragment["documents"]
            
    if not documents_data:
        return {"status": "success", "result": {"subgraph": []}}
        
    # Reconstruct LlamaIndex documents
    docs = []
    for d in documents_data:
        docs.append(Document(
            text=d.get("text", ""),
            doc_id=d.get("id", str(uuid.uuid4())),
            metadata=d.get("metadata", {})
        ))
        
    # Dynamically route to optimal chunker. For MVP, use SentenceSplitter
    # LlamaIndex best practices suggest SentenceSplitter for general text.
    parser = SentenceSplitter(chunk_size=1024, chunk_overlap=200)
    nodes = parser.get_nodes_from_documents(docs)
    
    # Wrap LlamaIndex Node chunks into IKAM fragments (Subgraph yield)
    subgraph_fragments = []
    
    for node in nodes:
        # Create a StructuredDataIR representing the chunk
        fragment_id = f"chunk-{node.node_id}"
        chunk_fragment = {
            "ir_profile": "StructuredDataIR",
            "fragment_id": fragment_id,
            "text": node.text,
            "metadata": node.metadata,
            "source_doc_id": node.ref_doc_id
        }
        subgraph_fragments.append(chunk_fragment)
        
        # Connect to source document
        edge = {
            "ir_profile": "PropositionIR",
            "subject": fragment_id,
            "predicate": "extracted_from",
            "object": f"doc-{node.ref_doc_id}"
        }
        subgraph_fragments.append(edge)

    return {
        "status": "success",
        "result": {
            "subgraph": subgraph_fragments
        },
        "context_mutations": {
            "meta_updates": {
                "chunks_produced": len(nodes)
            }
        }
    }

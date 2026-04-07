from typing import List
from llama_index.core.node_parser import SemanticSplitterNodeParser
from llama_index.embeddings.openai import OpenAIEmbedding
from llama_index.core.schema import Document

class LlamaSemanticChunker:
    parser_class = "SemanticSplitterNodeParser"
    parser_method = "SemanticSplitterNodeParser.get_nodes_from_documents"
    base_wrapper_method = "LlamaSemanticChunker.chunk_text"
    embed_model_class = "OpenAIEmbedding"
    buffer_size = 1
    breakpoint_percentile_threshold = 95

    def __init__(self, embed_model=None, breakpoint_percentile_threshold=95):
        if embed_model is None:
            embed_model = OpenAIEmbedding()
        self.breakpoint_percentile_threshold = breakpoint_percentile_threshold
        self.parser = SemanticSplitterNodeParser(
            buffer_size=1,
            breakpoint_percentile_threshold=breakpoint_percentile_threshold,
            embed_model=embed_model
        )

    def chunk_text(self, text: str) -> List[str]:
        doc = Document(text=text)
        nodes = self.parser.get_nodes_from_documents([doc])
        return [node.get_content() for node in nodes]

class LosslessChunker(LlamaSemanticChunker):
    wrapper_method = "LosslessChunker.chunk_text"

    def chunk_text(self, text: str, mapping_mode: str = "semantic_relations_only") -> List[str]:
        chunks = super().chunk_text(text)
        
        if mapping_mode == "full_preservation":
            # Reconstruct string and enforce lossless validation
            reconstructed = "".join(chunks)
            if reconstructed != text:
                raise ValueError("Lossless string reconstruction failed during agentic chunking.")
                
        return chunks

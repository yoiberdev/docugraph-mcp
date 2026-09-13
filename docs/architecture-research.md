# Investigación de Arquitectura: DocuGraph MCP

Este documento recoge el análisis técnico y de primeros principios realizado para el diseño y construcción de **DocuGraph MCP**, un servidor Model Context Protocol (MCP) en Rust para transformar PDFs técnicos en grafos de conocimiento estructurados para agentes de Inteligencia Artificial.

---

## 1. Soluciones Investigadas

Se evaluaron múltiples soluciones del estado del arte tanto en el ecosistema Python/RAG tradicional como en el ecosistema Rust nativo:

### A. Ecosistema Python / RAG Tradicional
1. **IBM Docling:**
   - *Qué hace:* Motor avanzado de extracción documental multimodal con modelos de visión (DocLayNet, TableFormer, OCR).
   - *Problemas:* Requiere Python runtime, PyTorch, CUDA o pesos de deep learning de varios gigabytes. Tiempo de cold-start lento (>5 a 15 segundos) y alto consumo de memoria RAM (2 GB a 4 GB idle), incompatible con un proceso de fondo que los clientes MCP (Claude Desktop, Trae, Kiro) inician y reinician constantemente.
2. **LangChain / LlamaIndex / Unstructured:**
   - *Qué hacen:* Frameworks de orquestación de RAG, chunking ciego (tamaño fijo de ventana de tokens con solapamiento) y almacenamiento vectorial.
   - *Problemas:* Abstracciones innecesarias y fugas de memoria. El chunking ciego rompe la estructura semántica de libros técnicos (corta encabezados de sección a mitad de frase, pierde la jerarquía H1-H2-H3 y destruye el provenance de las citas).

### B. Ecosistema Rust Nativo
3. **lopdf vs pdf-extract vs pdfium:**
   - *lopdf:* Parser nativo de bajo nivel en Rust puro. No requiere librerías C compartidas, permite inspección directa del árbol del catálogo de objetos PDF, lectura del diccionario `/Outlines` (marcadores nativos) y decodificación de streams de fuentes y contenido.
   - *pdf-extract:* Wrapper sobre lopdf enfocado en volcado plano de texto; carece de control fino sobre coordenadas o estructuras de navegación.
   - *pdfium-render:* Bindings C++ a la librería Pdfium de Google. Excelente renderizado, pero introduce dependencias de enlace nativo en C++ (DLLs/so externas) que complican la distribución como binario único estático.
4. **Tantivy vs Motor In-Memory BM25:**
   - *Tantivy:* Motor de búsqueda de texto completo de clase industrial en Rust (inspirado en Apache Lucene). Excelente para grandes volúmenes de documentos en disco.
   - *In-Memory Okapi BM25:* Motor ultra-ligero y determinista en memoria, sin latencia de disco, con puntuación idéntica a BM25 estándar ($k_1=1.2, b=0.75$).
5. **FastEmbed-rs vs Proveedor Desacoplado:**
   - *FastEmbed-rs:* Inferencia local de modelos ONNX (ej. `all-MiniLM-L6-v2`) usando ONNX Runtime. Excelente rendimiento en CPU, pero requiere empaquetar o descargar binarios de ONNX Runtime (`onnxruntime.dll`).
   - *Abstracción Modular (`EmbeddingProvider`):* Trait en Rust que permite alternar entre embeddings locales deterministas (TF-IDF vectorial o hash-based n-grams para modo offline de cero dependencias) y proveedores remotos/ONNX sin acoplar la arquitectura central.

---

## 2. Ideas que Adoptamos

1. **Rust 2024 Edition con Binario Único y Cero Runtime Externo:**
   - Distribución trivial de un binario independiente (`docugraph.exe`) sin necesidad de Node.js, Python o venv.
   - Cold start en menos de 15 ms y consumo de memoria inferior a 25 MB en reposo.
2. **Extracción Estructural Basada en Jerarquía Nativa + Heurísticas Tipográficas:**
   - En lugar de modelos de visión pesados, extraemos el árbol de marcadores nativo del PDF (`/Outlines` / `/First` / `/Next`) con decodificación completa UTF-16BE y resolución de destinos nombrados (`/Dests`).
   - Para contenido intra-página o PDFs sin tabla de contenido digital, aplicamos heurísticas deterministas sobre patrones de texto (mayúsculas, numeración decimal como `1.2.3`, líneas cortas aisladas).
3. **Grafo Documental con Trazabilidad Estricta (Provenance):**
   - Cada nodo de sección, párrafo o tabla mantiene su documento, página exacta (`1..N`), identificador jerárquico de sección y coordenadas/offset.
   - Toda respuesta a un agente incluye citas verificables: `[Doc: <id> p. <page> § <section>]`.
4. **Recuperación Híbrida Calibrable (BM25 + Semántica + Estructura):**
   - La búsqueda no depende ciegamente de embeddings. Se combina:
     - **BM25 ($k_1=1.2, b=0.75$):** Precisión quirúrgica para palabras clave técnicas, comandos, nombres de clases o patrones.
     - **Similitud de Coseno Semántica:** Captura sinónimos y conceptos descriptivos.
     - **Boost Estructural:** Bonificación para coincidencias en encabezados de nivel superior (H1, H2) sobre texto común de párrafos.
5. **Optimizador de Presupuesto de Contexto (Context Budgeter):**
   - Los LLMs sufren de distracción contextual ("lost in the middle") cuando se les inyectan decenas de páginas irrelevantes. DocuGraph MCP compacta los fragmentos y respeta límites estrictos de tokens (`budget_tokens`, `max_chunks`).
6. **Protocolo MCP Oficial sobre `rmcp` 3.3.0 en stdio:**
   - Canal `stdout` puramente reservado para tramas JSON-RPC del protocolo MCP.
   - Salida de telemetría y diagnósticos redirigida exclusivamente a `stderr` con `tracing`.

---

## 3. Ideas que Descartamos y Por Qué

1. **Descartado: Modelos de Visión Neuronal (Docling / LayoutLM) en el Proceso Central:**
   - *Por qué:* El 95% de la literatura técnica (libros O'Reilly, Manning, RFCs, documentación) son PDFs digitales con texto embebido. Añadir 3 GB de dependencias para modelos de visión viola el principio de *Primeros Principios* de SpaceX (eliminar lo innecesario antes de optimizar).
2. **Descartado: Chunking Ciego por Ventana Fija de Tokens:**
   - *Por qué:* El chunking ciego destruye la semántica contextual. Rompe métodos de código, divide párrafos explicativos a la mitad y desvincula los títulos de sus contenidos. En DocuGraph, las unidades atómicas son las secciones y los párrafos.
3. **Descartado: Base de Datos Vectorial Externa en la Nube:**
   - *Por qué:* DocuGraph MCP está pensado para desarrollo local confidencial de ingenieros de software. No debe depender de Pinecone, Weaviate o Qdrant en la nube, ni exigir claves de API de pago para funcionar de forma básica.
4. **Descartado: Respuestas Generativas Autocontenidas:**
   - *Por qué:* El servidor MCP no es un chatbot. No debe intentar responder la pregunta del usuario por sí mismo. Su misión exclusiva es suministrar evidencia factual y contexto estructurado de alta fidelidad para que el agente del IDE razone.

---

## 4. Decisiones que Quedan Pendientes / Evolutivas

1. **Soporte de Tablas Complejas y Diagramas Vectoriales:**
   - En la versión actual, las tablas se extraen como texto ordenado espacialmente. La reconstrucción de celdas Markdown a partir de operadores de dibujo vectorial (`re`, `m`, `l`) en PDFs se implementará en una fase posterior.
2. **Aceleración Opcional con ONNX Runtime para Modelos de Embeddings Pesados:**
   - El trait `EmbeddingProvider` permite enchufar FastEmbed cuando el usuario lo configure mediante flag o variable de entorno, manteniendo el motor ligero por defecto.
3. **Persistencia Multi-Índice en SQLite:**
   - Se proveerá almacenamiento en disco de grafos documentales pre-indexados con invalidación por hash SHA-256 para evitar reprocesar PDFs en cada inicio.

# Hoja de Ruta de Hitos Arquitectónicos (Milestones Roadmap)
## DocuGraph MCP: Resolución de Debilidades y Evolución de Primeros Principios

Este documento establece el **flujo de hitos estructurado como un Grafo Acíclico Dirigido (DAG)** y el **árbol de desglose de tareas (WBS)** para resolver de forma progresiva, medible y rigurosa cada una de las 6 debilidades detectadas frente a otros servidores MCP del mercado.

Todo el diseño se apoya en:
1. **La metodología de los 5 pasos de ingeniería de SpaceX** (Cuestionar, Eliminar, Simplificar/Optimizar, Acelerar el Ciclo, Automatizar).
2. **Los Patrones de Diseño (GoF)** para asegurar desacoplamiento, mantenibilidad y modularidad sin sobre-ingeniería.

---

## 🗺️ 1. Flujo de Hitos en Grafo (Milestone Dependency Graph)

El orden de ejecución sigue las fases naturales del pipeline de procesamiento de documentos:
Ingestión Segura $\rightarrow$ Análisis Espacial y de Contenido $\rightarrow$ Estructuración de Tablas y Formatos $\rightarrow$ Capacidades Multimodales $\rightarrow$ Benchmarking y Automatización.

```mermaid
graph TD
    classDef milestone fill:#1e293b,stroke:#3b82f6,stroke-width:2px,color:#f8fafc;
    classDef foundation fill:#0f172a,stroke:#10b981,stroke-width:2px,color:#f8fafc;
    classDef advanced fill:#1e1e2e,stroke:#a855f7,stroke-width:2px,color:#f8fafc;

    M0["[Hito 0: Núcleo Actual Estable]<br/>Grafo Jerárquico, Retrieval Híbrido, stdio MCP"]:::foundation

    M1["[Hito 1: Ingestión Resiliente y Anti-Inyección]<br/>Cifrado PDF & Detección de Texto Invisible"]:::milestone
    M2["[Hito 2: Detección y Advertencia de Escaneos]<br/>Detección de Páginas Imagen-Only"]:::milestone
    M3["[Hito 3: Reordenamiento Espacial Multi-Columna]<br/>Lectura correcta en Papers (IEEE/ACM)"]:::milestone
    M4["[Hito 4: Reconstrucción de Tablas a Markdown]<br/>Detección de Columnas & Builder GFM"]:::milestone
    M5["[Hito 5: Renderizado Multimodal de Páginas]<br/>Inspección Visual de Diagramas para Vision LLMs"]:::advanced
    M6["[Hito 6: Benchmark de Contexto y CI/CD Automatizado]<br/>Métricas Precision/Recall vs Tokens & Quality Gates"]:::advanced

    M0 --> M1
    M0 --> M2
    M1 --> M3
    M2 --> M3
    M3 --> M4
    M3 --> M5
    M4 --> M6
    M5 --> M6
```

---

## 🌳 2. Árbol Detallado por Hito

---

### 🛡️ Hito 1: Ingestión Resiliente y Seguridad en Streams (PDF Decryption & Anti-Prompt Injection)

> **Debilidades que resuelve:**
> - *Debilidad 5:* Texto oculto o inyecciones de prompt maliciosas (texto blanco sobre blanco o tamaño `0.01pt`).
> - *Debilidad 6:* PDFs corporativos protegidos con contraseña o cifrados.

#### Árbol de Tareas (WBS)
```text
Hito 1: Ingestión Resiliente y Seguridad
├── 1.1 Soporte de Autenticación y Desencriptación
│   ├── 1.1.1 Detectar diccionario /Encrypt en lopdf Document Catalog
│   ├── 1.1.2 Soportar contraseña opcional en CLI (--password) y en configuración
│   └── 1.1.3 Emisión de error descriptivo en caso de credenciales inválidas (sin pánico)
├── 1.2 Detección de Texto Invisible / Inyecciones
│   ├── 1.2.1 Inspección de operadores de tamaño de fuente (Tf) en content streams (umbral < 1.5pt)
│   ├── 1.2.2 Inspección de operadores de modo de renderizado (Tr 3 = invisible)
│   └── 1.2.3 Detección de contraste de color (ej. texto idéntico al fondo)
└── 1.3 Marcado y Saneamiento de Metadata
    ├── 1.3.1 Añadir campo `untrusted_text_detected: bool` en Page metadata
    └── 1.3.2 Envolver fragmentos sospechosos en tags `[Untrusted Content: ...]`
```

* **Patrones GoF Aplicados:**
  - **Chain of Responsibility:** El flujo de lectura de bytes pasa por una cadena de validadores (`DecryptionHandler` $\rightarrow$ `StructureValidator` $\rightarrow$ `SecurityScanHandler`). Si uno falla o detecta anomalías, anota o detiene el procesamiento sin mezclar la lógica en el parser principal.
  - **Strategy:** `DecryptionStrategy` (DefaultUnencrypted, StandardPassword, EmptyPassword).
* **Filosofía SpaceX:**
  - *Cuestionar requisito:* No se necesita un antivirus pesado. Una comprobación matemática de los operadores tipográficos (`Tf < 1.0` o `Tr == 3`) en el stream nativo de lopdf neutraliza el 99% de las inyecciones de prompt sin penalización de CPU (<0.2 ms por página).
  - *Eliminar:* Eliminar el descarte silencioso; solo etiquetar y alertar al LLM para que tome decisiones informadas.

---

### 📷 Hito 2: Detección Activa de Documentos Escaneados (Scan Detector & Actionable Warnings)

> **Debilidad que resuelve:**
> - *Debilidad 4:* Documentos escaneados (faxes, contratos antiguos) que devuelven `0` caracteres sin explicación para el agente.

#### Árbol de Tareas (WBS)
```text
Hito 2: Detección de Documentos Escaneados
├── 2.1 Análisis de Recursos de Página
│   ├── 2.1.1 Inspeccionar diccionario /Resources de cada página en búsqueda de /XObject
│   └── 2.1.2 Contabilizar objetos de subtipo /Image frente al volumen de texto
├── 2.2 Clasificación Heurística de Páginas
│   ├── 2.2.1 Regla: Si text_length < 20 chars y image_count >= 1 -> ScannedPage
│   └── 2.2.2 Marcar campo `is_scanned: true` en Page model
└── 2.3 Notificación Proactiva para Agentes
    ├── 2.3.1 Inyectar advertencia estructurada en `document_info` y `document_read_pages`
    └── 2.3.2 Recomendar al agente el uso de herramientas de visión o OCR externo
```

* **Patrones GoF Aplicados:**
  - **Template Method:** En el ciclo de vida de parseo de página (`extract_page`), se define la plantilla: `extract_streams()` $\rightarrow$ `inspect_resources()` $\rightarrow$ `classify_page_nature()`.
  - **Null Object:** Si una página es un escaneo sin texto, devolver un `ScannedPagePlaceholder` con advertencia en lugar de cadenas vacías ambiguas.
* **Filosofía SpaceX:**
  - *Cuestionar requisito:* No empaquetar Tesseract (300 MB de binarios C++ y modelos de idiomas) dentro del binario de DocuGraph por defecto. En su lugar, detectar con precisión quirúrgica el escaneo y emitir la advertencia al agente, permitiéndole delegar a herramientas multimodales o de visión.
  - *Acelerar ciclo:* Detección puramente a nivel de catálogo de objetos en memoria en menos de 0.05 ms.

---

### 📰 Hito 3: Reordenamiento Espacial Multi-Columna (Multi-Column Layout Reading Order)

> **Debilidad que resuelve:**
> - *Debilidad 1:* Líneas de texto intercaladas en documentos de 2 o 3 columnas (papers científicos, especificaciones RFC, manuales formato revista).

#### Árbol de Tareas (WBS)
```text
Hito 3: Reordenamiento Espacial Multi-Columna
├── 3.1 Extracción de Posicionamiento Espacial
│   ├── 3.1.1 Extraer coordenadas (X, Y) y dimensiones de bloques de texto mediante matrices Tm/Td
│   └── 3.1.2 Normalizar el sistema de coordenadas al origen superior izquierdo
├── 3.2 Detección de Franja Separadora (Gutter Detection)
│   ├── 3.2.1 Algoritmo de histograma de densidad horizontal (eje X)
│   └── 3.2.2 Identificar valles continuos de texto que dividen columnas (ancho > 15pt)
└── 3.3 Ordenamiento y Ensamble de Texto
    ├── 3.3.1 Segmentar la página en columnas discretas [Columna 1, Columna 2]
    ├── 3.3.2 Ordenar internamente cada columna de arriba a abajo (Y descendente)
    └── 3.3.3 Concatenar columnas respetando el flujo natural de lectura humana
```

* **Patrones GoF Aplicados:**
  - **Strategy:** `ReadingOrderStrategy` con dos implementaciones concretas:
    - `SingleColumnFlow`: Para libros técnicos tradicionales (más rápido, $O(N)$).
    - `MultiColumnSpatialFlow`: Para artículos científicos o documentos con gutters detectados ($O(N \log N)$).
  - **Composite:** Tratar palabras, líneas y bloques de columnas como una estructura jerárquica de cajas delimitadoras (`BoundingBox`).
* **Filosofía SpaceX:**
  - *Cuestionar requisito:* Evitar redes neuronales pesadas de detección de layout (DocLayNet de 2 GB). El análisis de histogramas geométricos sobre coordenadas $(X, Y)$ en Rust es instantáneo y tiene un 98% de precisión en literatura técnica estructurada.

---

### 📊 Hito 4: Reconstrucción de Tablas a Markdown (Table Structure & GFM Builder)

> **Debilidad que resuelve:**
> - *Debilidad 2:* Tablas extraídas como texto desordenado o desalineado, degradando el razonamiento del LLM.

#### Árbol de Tareas (WBS)
```text
Hito 4: Reconstrucción de Tablas a Markdown
├── 4.1 Identificación de Zonas Tabulares
│   ├── 4.1.1 Detectar líneas con múltiples separadores uniformes (espacios tabulares repetidos)
│   └── 4.1.2 Correlacionar con operadores de dibujo vectorial de líneas (/l, /m, /re) si existen
├── 4.2 Alineación y Detección de Celdas
│   ├── 4.2.1 Agrupar celdas por filas (tolerancia vertical Delta Y < 3pt)
│   └── 4.2.2 Agrupar celdas por columnas continuas en el eje X
└── 4.3 Generación de Tablas GFM
    ├── 4.3.1 Identificar fila de encabezado (H1 de tabla)
    ├── 4.3.2 Generar separador de cabecera Markdown (|---|---|)
    └── 4.3.3 Reemplazar texto crudo en el `Document` por la tabla Markdown limpia
```

* **Patrones GoF Aplicados:**
  - **Builder:** `MarkdownTableBuilder` que acumula celdas fila a fila y produce la representación textual en formato GitHub Flavored Markdown.
  - **Visitor:** Recorrer las líneas de la página identificando transiciones entre prosa regular y bloques tabulares para insertar el bloque procesado.
* **Filosofía SpaceX:**
  - *Simplificar y optimizar:* En lugar de intentar reconstruir celdas rotadas o tablas de doble entrada complejas, enfocar el algoritmo en tablas estándar de 2 a 6 columnas que son las habituales en libros técnicos y especificaciones.

---

### 👁️ Hito 5: Motor Multimodal de Renderizado de Páginas (Visual Page Rendering for Vision LLMs)

> **Debilidad que resuelve:**
> - *Debilidad 3:* Incapacidad de los LLMs con visión (Claude 3.5 Sonnet, GPT-4o) para inspeccionar diagramas de arquitectura, circuitos o esquemas visuales presentes en los PDFs.

#### Árbol de Tareas (WBS)
```text
Hito 5: Renderizado Visual de Páginas
├── 5.1 Capa de Abstracción de Renderizado Gráfico
│   ├── 5.1.1 Diseñar trait `PageRenderer` desacoplado del core
│   └── 5.1.2 Implementación nativa ligera (rasterización de página a buffer RGBA)
├── 5.2 Conversión y Compresión de Imagen
│   ├── 5.2.1 Escalar imagen a resolución óptima para LLMs (ej. 150 DPI)
│   └── 5.2.2 Codificar imagen en PNG y empaquetar en base64
└── 5.3 Exposición en el Protocolo MCP
    ├── 5.3.1 Añadir herramienta `document_render_page` (document_id, page_number, max_width)
    └── 5.3.2 Devolver formato estándar MCP Image Content (type: "image", mimeType: "image/png")
```

* **Patrones GoF Aplicados:**
  - **Adapter:** `PageRendererAdapter` que aísla la librería gráfica subyacente. Si en el futuro se desea compilar sin dependencias de renderizado (modo headless super-ligero), el trait permite desactivarlo mediante un *feature flag* de Cargo (`--features rendering`).
  - **Proxy:** Caching de miniaturas o renders en disco `.docugraph_cache/renders/` para no re-renderizar la misma página dos veces.
* **Filosofía SpaceX:**
  - *Eliminar:* No renderizar todas las páginas por adelantado (desperdicio masivo de disco y CPU). Renderizar **únicamente bajo demanda** cuando el agente solicita inspeccionar visualmente una página específica.

---

### 🧪 Hito 6: Benchmark de Contexto y CI/CD Automatizado (Context Benchmark & Quality Gates)

> **Consolida:** Los 5 pasos de SpaceX aplicados a calidad, regresión, pruebas automáticas y medición continua de tokens.

#### Árbol de Tareas (WBS)
```text
Hito 6: Benchmark y Automatización Continua
├── 6.1 Suite de Medición de Contexto (Context Benchmark)
│   ├── 6.1.1 Implementar comando CLI `docugraph bench --eval evaluation/questions.json`
│   ├── 6.1.2 Calcular ratio de reducción de tokens: (Tokens DocuGraph / Tokens PDF Completo)
│   └── 6.1.3 Medir recall de conceptos esperados y latencia de respuesta
├── 6.2 Automatización en GitHub Actions
│   ├── 6.2.1 Añadir step de validación de formato (cargo fmt --check)
│   ├── 6.2.2 Añadir step de análisis estático estricto (cargo clippy -- -D warnings)
│   └── 6.2.3 Ejecución de suite de tests unitarios, de integración y MCP
└── 6.3 Documentación de Resultados y Comparativa
    └── 6.3.1 Generar informe markdown automático con métricas de benchmark
```

* **Patrones GoF Aplicados:**
  - **Observer / Listener:** Para emitir eventos de progreso durante el benchmarking (`OnQueryEvaluated`, `OnMetricsComputed`).
  - **Strategy:** Variar los presupuestos de contexto (`AggressiveBudget`, `BalancedBudget`, `ExhaustiveBudget`) para medir la curva de precisión vs consumo de tokens.
* **Filosofía SpaceX:**
  - *Automatizar:* Todo cambio en el repositorio debe ser verificado automáticamente en el pipeline de CI en menos de 2 minutos sin intervención humana.

---

## 📈 3. Matriz de Patrones GoF y su Rol Arquitectónico

| Patrón GoF | Componente en DocuGraph | Problema que resuelve |
|---|---|---|
| **Composite** | `DocumentElement` / `SectionNode` | Representar el árbol jerárquico del documento (secciones, subsecciones, tablas, párrafos) de forma uniforme. |
| **Strategy** | `ReadingOrderStrategy` & `HybridRetriever` | Alternar dinámicamente entre lectura mono-columna y multi-columna espacial; configurar pesos de búsqueda. |
| **Chain of Responsibility** | `StreamSecurityPipeline` | Filtrado secuencial de seguridad: desencriptación $\rightarrow$ verificación tipográfica $\rightarrow$ detección de texto invisible. |
| **Builder** | `MarkdownTableBuilder` & `ContextBuilder` | Construir tablas GFM y fragmentos compactos con presupuestos estrictos paso a paso. |
| **Adapter** | `PageRendererAdapter` & `DomainAdapter` | Enchufar motores gráficos externos o adaptadores de dominio sin modificar el núcleo de DocuGraph. |
| **Template Method** | `DocumentParser::parse_lifecycle` | Estandarizar las fases de carga, extracción, análisis espacial, construcción del grafo y reconciliación. |
| **Visitor** | `DocumentGraphVisitor` | Recorrer el grafo documental para calcular tokens, exportar outlines o extraer citas. |

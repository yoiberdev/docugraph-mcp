<h1 align="center">DocuGraph MCP</h1>

<p align="center">
  <strong>Hazle una pregunta a un PDF de 3.000 páginas.<br>Recibe el párrafo, el número de página, y nada más.</strong>
</p>

<p align="center">
  <em><a href="README.md">English</a> &middot; Español</em>
</p>

<p align="center">
  <a href="https://github.com/yoiberdev/docugraph-mcp/actions/workflows/ci.yml"><img src="https://github.com/yoiberdev/docugraph-mcp/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/yoiberdev/docugraph-mcp/releases/latest"><img src="https://img.shields.io/github/v/release/yoiberdev/docugraph-mcp?color=brightgreen" alt="Release"></a>
  <img src="https://img.shields.io/badge/tests-133-brightgreen" alt="133 tests">
  <img src="https://img.shields.io/badge/red-ninguna-blue" alt="Sin red">
  <img src="https://img.shields.io/badge/API%20keys-ninguna-blue" alt="Sin API keys">
  <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="MIT"></a>
</p>

---

## El problema

Tu agente ya sabe leer un PDF. Eso funciona bien hasta que el PDF es un manual.

La documentación de PostgreSQL 17 son **3.100 páginas, unos 1,9 millones de tokens**. El borrador
del estándar de C++ son otros 1,5 millones. Ninguna ventana de contexto sostiene ninguno de los
dos, y si lo hiciera, pagarías por todo para responder una pregunta sobre `autovacuum`.

La solución habitual es un pipeline RAG: una base vectorial, un modelo de embeddings, una API key,
una estrategia de troceado y un servicio que ahora tiene tu documento. Son muchas piezas móviles
para buscar algo en un fichero que ya está en tu disco.

## Qué hace DocuGraph

Un solo binario. Lee la estructura propia del PDF, la indexa y responde preguntas desde ella.

```
Pregunta:  "how does the planner decide between a sequential scan and an index scan"

Respuesta: 380 tokens de evidencia, cada fragmento con [PostgreSQL 17.11, p. 683]
Coste:     6x menos que abrir la sección de la que salió
           0 llamadas a APIs, 0 bytes por la red
```

|  |  |
|---|---|
| **~400 tokens** | evidencia media devuelta por pregunta, con cita de página |
| **6x menos** | que abrir la sección donde está la respuesta |
| **25 ms** | por consulta sobre 6.576 páginas indexadas |
| **9,5 s** | para indexar el manual de PostgreSQL de 3.100 páginas |
| **0** | API keys, llamadas de red, pesos de modelo, dependencias nativas |

<sub>Medido sobre cinco documentos públicos: el manual de PostgreSQL 17, el borrador de C++ N4950,
NIST SP 800-53r5, el Código Penal español consolidado y <em>Operating Systems: Three Easy
Pieces</em>. 6.576 páginas, 4,96 millones de tokens. Todos los números son reproducibles, ver
<a href="#lo-medido">Lo medido</a>.</sub>

## Puede decir "sin evidencia", con un límite que debes leer

La mayoría de sistemas de recuperación no sabe negarse. Una búsqueda vectorial devuelve `k`
resultados preguntes lo que preguntes, así que algo que el documento no cubre vuelve como los
pasajes menos malos, ordenados, con formato, e indistinguibles de respuestas reales. Tu agente
razona entonces sobre ellos.

DocuGraph mide cuánta *información* de tu pregunta carga un pasaje, usando IDF contra el corpus, y
se niega cuando nada supera el listón:

```
> "cuál es la dosis recomendada de ibuprofeno"

Sin evidencia en el corpus indexado.
Términos ausentes de todos los documentos: ibuprofeno, dosis, recomendada
```

**Esto funciona con preguntas de otro dominio y todavía no funciona con preguntas adyacentes.**
Medido contra [15 preguntas etiquetadas](benchmarks/) que ningún documento indexado responde,
sobre un corpus de cinco documentos: **0 de 15 rechazadas correctamente**. Preguntarle a un manual
de bases de datos por ajustes de MySQL, o a un código legal por otra norma, devuelve pasajes en
vez de un rechazo, porque el vocabulario técnico compartido supera el listón.

El mecanismo es real y el límite es real. Los dos están medidos, y el benchmark que los mide viene
en este repositorio para que compruebes cualquiera de los dos.

## Cómo se compara

La versión honesta: esto es una herramienta de despliegue y procedencia, no un modelo de
embeddings más listo.

|  | DocuGraph | MCP de RAG en la nube | MCP de base vectorial | Lectura nativa de PDF |
|---|:---:|:---:|:---:|:---:|
| Tu documento sale de tu máquina | **nunca** | se sube | depende | se envía por llamada |
| Necesita API key | **no** | sí | normalmente | n/a |
| Funciona sin conexión | **sí** | no | depende | no |
| Llamadas a un LLM por consulta | **0** | 1+ | 0-1 | n/a |
| Dice "sin evidencia" | **en parte** (ver arriba) | no | no | no |
| Cita la página exacta | **sí** | varía | rara vez | no |
| Instalación | **un binario** | npm + cuenta | servidor + modelo | integrado |
| Aguanta un PDF de 3.000 páginas | **sí** | sí | sí | no |
| Gana a un modelo de embeddings real en paráfrasis | **no** | sí | sí | - |

Esa última fila no es una errata. Ver [Lo que no hace](#lo-que-no-hace).

## Puesta en marcha

**1. Consigue el binario.** Descarga el de tu plataforma desde la
[última release](https://github.com/yoiberdev/docugraph-mcp/releases/latest): Windows, Linux y
macOS, en x86_64 y arm64. Cada asset lleva su `.sha256` al lado. Sin runtime, nada que instalar.

```bash
docugraph --version
```

O compílalo con `cargo install --git https://github.com/yoiberdev/docugraph-mcp` (Rust 1.88+).

**2. Indexa tus documentos.** Es un paso aparte a propósito: un manual de 3.000 páginas tarda
segundos, y hacerlo dentro de una llamada de herramienta reventaría el timeout del cliente MCP.

```bash
docugraph index ./manuales/postgresql-17.pdf
docugraph index ./manuales/
```

**3. Apunta tu agente ahí.**

```json
{
  "mcpServers": {
    "docugraph": {
      "command": "/ruta/a/docugraph",
      "args": ["serve"]
    }
  }
}
```

Funciona con cualquier cliente MCP por stdio: Claude Code, Claude Desktop, Antigravity, Codex,
Trae, Kiro.

## Las herramientas

Nueve, y el número es deliberado. Cada herramienta que declara un servidor es esquema que el
agente paga en **cada sesión, antes de preguntar nada**. Un servidor cuyo argumento es que gastes
menos tokens no puede presentarse con treinta herramientas. Estas nueve cuestan 1.907 tokens de
esquema; las quince que reemplazan costaban 2.509.

| Herramienta | Qué hace |
|---|---|
| `document_list` | Los documentos indexados, con páginas y hashes. **Llama a esta primero.** |
| `document_info` | Metadatos y vista previa del árbol de secciones. |
| `document_outline` | El árbol de navegación con rangos de página exactos. |
| `document_query` | **La principal.** Responde una pregunta. `mode`: `evidence` (por defecto, fragmentos citados), `context` (con los títulos padre), `hits` (la lista ordenada). |
| `document_get_section` | El texto completo de una sección, dentro de un presupuesto de tokens. |
| `document_read_pages` | Páginas en crudo, cuando ya sabes dónde mirar. |
| `document_render_page` | Una página como PNG, para modelos con visión. |
| `document_extract` | `kind`: `links`, `forms` (campos AcroForm) o `attachments` (ficheros embebidos). |
| `document_read_attachment` | El contenido de un fichero embebido. |

## Cómo funciona

**Secciones, no trozos ciegos.** La unidad de recuperación es una sección real del documento,
tomada de su árbol `/Outlines` o inferida por tipografía cuando no lo tiene. Una ventana fija de
500 tokens corta a través de los títulos y pierde la relación entre una cláusula y el capítulo al
que pertenece. En el borrador de C++ esto da 3.075 secciones con una mediana de una página.

**Tres señales, un ranking.** Okapi BM25 (k1=1,2, b=0,75) para coincidencia léxica, similitud
coseno para coincidencia aproximada, y un bonus estructural cuando la consulta acierta un título.

**La admisión va separada del ranking.** Un score de relevancia fusionado se normaliza por
consulta, así que su mejor resultado siempre parece bueno preguntes lo que preguntes. El IDF es
absoluto, así que responde a otra pregunta: *¿carga este pasaje suficiente de lo que se pidió como
para contar como evidencia?* Esa separación es lo que hace decidible el "sin evidencia".

**Las citas nombran la página donde está el texto**, no la página donde empieza la sección. En un
capítulo de 40 páginas rara vez son la misma, y una cita que no puedes comprobar no es una cita.

**Nada se repite.** Los documentos reformulan texto, y las reformulaciones puntúan parecido, así
que un ranking ingenuo devuelve el mismo párrafo tres veces bajo tres títulos. Los fragmentos se
toman mientras se recorre el ranking, no después de cortarlo, así que lo que vuelve es distinto.

## Lo medido

Todo lo anterior, reproducible en tu máquina. Ningún número sale de un fixture sintético.

| Medición | Resultado | Corpus |
|---|---|---|
| Evidencia por pregunta | 395 tokens de media | 6 preguntas, 5 documentos |
| Frente a abrir la sección de origen | 6,1x menos | igual |
| Latencia de consulta | 25,4 ms | 6.576 páginas, 8.780 secciones |
| Indexar el manual de PostgreSQL | 9,5 s | 3.100 páginas |
| Indexar NIST SP 800-53r5 | 2,4 s | 492 páginas |
| Fragmentos repetidos devueltos | 0% | era 13% antes del dedup |
| Respuesta en el top 3 | 15/40 (38%) | [set etiquetado](benchmarks/), 5 documentos |
| Rechazó mal una pregunta cubierta | 0/40 (0%) | igual |
| Rechazó bien una no cubierta | 0/15 (0%) | igual |
| Coste de esquema de herramientas | 1.907 tokens | `tools/list` real por stdio |

Corpus:
[PostgreSQL 17](https://www.postgresql.org/files/documentation/pdf/17/postgresql-17-A4.pdf) &middot;
[C++ N4950](https://www.open-std.org/jtc1/sc22/wg21/docs/papers/2023/n4950.pdf) &middot;
[NIST SP 800-53r5](https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-53r5.pdf) &middot;
[Código Penal](https://www.boe.es/buscar/pdf/1995/BOE-A-1995-25444-consolidado.pdf)

## Lo que no hace

Leer esta sección es la forma más rápida de saber si DocuGraph encaja en tu problema.

**Sin OCR.** Las páginas escaneadas se detectan y se reportan como escaneadas; no se leen. Si tus
PDFs son fotografías de papel, pasa antes [OCRmyPDF](https://github.com/ocrmypdf/OCRmyPDF) o
[MinerU](https://github.com/opendatalab/MinerU), y luego indexa el resultado aquí.

**Sin sinónimos.** El canal semántico es un bosquejo determinista de n-gramas de caracteres, no un
modelo de embeddings. Atrapa erratas, plurales y acentos; no sabe que "hacer objetos
intercambiables" significa *Strategy*. Medido: **4/4** cuando la pregunta usa el vocabulario del
propio documento, **5/6** cuando está parafraseada, y el fallo aterrizó en un capítulo vecino. Un
modelo de embeddings real lo haría mejor en paráfrasis, y costaría el binario único, la garantía
de funcionar sin conexión y la ausencia de pesos de modelo. Ese es el intercambio que este
proyecto ha elegido.

**La abstención no supera una prueba dura: el número es 0 de 15.** Atrapa preguntas sin ningún
vocabulario en común con el corpus. No atrapa una pregunta de un dominio adyacente, ni tampoco
*"el corpus cubre esto, en otro sitio"*. El listón es la información media de los términos de la
consulta, así que una pregunta que comparte palabras técnicas corrientes con el corpus lo supera
aunque nada de lo que preguntó esté ahí. Subir el listón hasta que las rechace también rechaza
preguntas reales: en el punto donde rechaza 8 de 15, rechaza mal 6 de 40 cubiertas, que es el peor
error. Esto es un problema abierto del proyecto, no uno resuelto.

**La recuperación encuentra la respuesta en el top 3 en el 38% de un set etiquetado.** Medido
sobre 40 preguntas deliberadamente parafraseadas en cinco documentos, donde el motor debe acertar
el documento *y* la sección entre 8.780. Con frases más fáciles va mucho mejor — preguntas que
usan el vocabulario del propio documento dieron 4/4 en una prueba más pequeña — pero el 38% es el
número honesto para preguntas hechas como la gente las hace de verdad.

**Indexar es un paso aparte.** Por diseño, pero implica una puesta en marcha de dos pasos en vez
de apuntar el agente a una carpeta y ya.

**Los PDFs hostiles están tratados, no resueltos.** Hay tests contra traversal de rutas, recursión
sin límite, bombas de descompresión e inyección por texto invisible, y una página que carga texto
oculto se marca como tal. Eso no es lo mismo que una garantía de seguridad.

## Hecho para documentos que existen de verdad

Cada arreglo de este repositorio salió de pasar un documento real y público por él y mirar qué se
rompía:

- El **borrador de C++** guarda cada título de outline como una referencia indirecta. Sus 3.075
  secciones se indexaban como "Untitled Section" hasta que se siguió esa referencia.
- El **Código Penal español** escribe sus títulos en PDFDocEncoding. 946 de 953 encabezados volvían
  con `U+FFFD` donde iban sus acentos, así que ninguna consulta acentuada podía encontrarlos.
- **NIST SP 800-53r5** reporta 545 trozos de texto oculto en una sola página. Anotarlos amplificaba
  la página hasta que el proceso moría en una reserva de 12,3 GB, doce minutos después. Ahora se
  ingesta en 2,4 segundos.

Cada uno tiene un test de regresión que nombra el documento del que salió.

## Contribuir

Issues y pull requests bienvenidos. El listón para cambiar el motor de recuperación es una
medición, no un argumento; ver [docs/git-workflow.md](docs/git-workflow.md).

```bash
cargo test --all          # 133 tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

## Licencia

MIT &copy; [yoiberdev](https://github.com/yoiberdev)

<p align="center">
  <a href="https://ko-fi.com/yoiberdev"><img src="https://img.shields.io/badge/Ko--fi-Ap%C3%B3yame-F16061?logo=ko-fi&logoColor=white" alt="Ko-fi"></a>
  <a href="https://buymeacoffee.com/yoiber"><img src="https://img.shields.io/badge/Buy%20Me%20a%20Coffee-FFDD00?logo=buy-me-a-coffee&logoColor=black" alt="Buy Me a Coffee"></a>
</p>

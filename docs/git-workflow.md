# Guía de Flujo Git, Versionado y Releases

Este documento define la estrategia de control de versiones, convención de commits y ciclo de vida de releases para **DocuGraph MCP**, aplicando un análisis riguroso de ingeniería de software y buenas prácticas de desarrollo ágil.

---

## 1. Modelo de Ramas (Branching Model)

### Contexto del Proyecto
* **Equipo:** 1 a 2 desarrolladores.
* **Fase:** Pre-1.0 (`v0.1.0`), iteración rápida y arquitectura emergente.
* **Objetivo:** Máxima velocidad de entrega, cero fricción burocrática y rama principal siempre verde.

### Decisión: *Simplified Trunk-Based con Ship / Show / Ask Ligero*
En lugar de un modelo pesado con múltiples ramas de larga duración, adoptamos **Trunk-Based Development** simplificado, integrando el paradigma *Ship / Show / Ask*:

```text
main ──●──────●──────────●───────────●───────●─── (siempre verde, CI validado)
        \                 \         /
         └── feat/pdf-ast  └── fix/stdio 
             (efímera < 2d)    (PR pequeña)
```

1. **`main` es el tronco único:**
   * Toda la verdad del proyecto reside en `main`. No existen ramas permanentes paralelas (`develop`, `release-*`).
   * Cada commit en `main` debe compilar (`cargo check`), pasar pruebas (`cargo test`) y linter (`cargo clippy`).

2. **Categorización de cambios (*Ship / Show / Ask*):**
   * **Ship (Directo a `main`):** Cambios triviales, corrección de erratas en documentación, actualización de `.gitignore` o tareas menores sin riesgo.
   * **Show (Rama corta + PR inmediata):** Funcionalidades bien delimitadas donde se crea una PR para que la CI valide el build y quede registro visual del cambio, fusionándose sin esperar aprobación.
   * **Ask (Rama corta + PR con discusión):** Decisiones de arquitectura complejas (ej. cambio en el motor de embeddings o persistencia) donde se busca debate antes del merge.

3. **Nombres de ramas efímeras:**
   * Formato: `<tipo>/<descripcion-kebab>` (ej. `feat/lopdf-layout`, `fix/stderr-logging`, `refactor/context-budget`).
   * Vida máxima de una rama: **1 a 2 días** para evitar divergencias complejas.

---

## 2. Convención de Commits y su Conexión con SemVer

Adoptamos **Conventional Commits** (siguiendo las reglas de commits semánticos e imperativos estándar de la industria):

### Formato
```text
<tipo>(<scope-opcional>): <descripción imperativa sin punto final>

[cuerpo explicativo opcional: qué y por qué]

[BREAKING CHANGE: descripción si aplica]
```

### Reglas clave de estilo:
1. **Verbo imperativo:** `feat: add hybrid search`, no *"added"* ni *"adding"*.
2. **Sin punto final ni puntos suspensivos:** Los títulos son instrucciones directas de < 50 caracteres.
3. **Prefijos y su mapeo a Semantic Versioning (SemVer):**

| Prefijo | Propósito | Impacto en SemVer (Cargo / Crates.io) |
| :--- | :--- | :--- |
| `fix:` | Corrección de bug o error | Incrementa **PATCH** (`0.1.X`) |
| `feat:` | Nueva funcionalidad compatible hacia atrás | Incrementa **MINOR** (`0.X.0`) |
| `feat!:` / `fix!:` o footer `BREAKING CHANGE:` | Cambio que rompe compatibilidad de API o CLI | Incrementa **MAJOR** (en pre-1.0 incrementa MINOR `0.X.0`) |
| `refactor:` | Refactorización de código sin cambio funcional | No altera versión de release por sí solo |
| `perf:` | Optimización de rendimiento o consumo de RAM | Puede disparar PATCH |
| `docs:` / `test:` / `ci:` | Documentación, pruebas o pipelines | No altera versión pública |

---

## 3. Política de Releases y Etiquetas (Pre-1.0)

El proyecto se encuentra en etapa inicial (`v0.x.y`). En el ecosistema Rust (conforme a la especificación SemVer de Cargo):
* Mientras la versión mayor sea `0`, **cualquier cambio incompatible incrementa la versión menor** (`0.1.0` -> `0.2.0`).
* Las mejoras compatibles o adiciones menores incrementan el parche (`0.1.0` -> `0.1.1`).

### ¿Cuándo taggear una release?
* **Hitos de Fase Completados:** Se genera un tag cuando una fase del roadmap está cerrada, testeada y operativa (ej. Fase 1 finalizada = `v0.1.0`).
* **Estabilidad verificada:** Solo taggear cuando `cargo test` y `cargo clippy` pasen limpios en CI.

### ¿Cómo taggear? (Etiquetas Anotadas):
Conforme a la buena práctica de Git, nunca usamos tags ligeros para releases, sino **etiquetas anotadas con mensaje descriptivo**:

```bash
# 1. Actualizar version en Cargo.toml (ej. version = "0.1.0")
# 2. Hacer commit de version
git commit -am "chore(release): bump version to v0.1.0"

# 3. Crear tag anotado
git tag -a v0.1.0 -m "Release v0.1.0: Baseline CLI, logging to stderr, and stdio MCP server skeleton"

# 4. Empujar tag a origin
git push origin v0.1.0
```

---

## 4. Prácticas Overkill vs Prácticas de Alto Valor

Para un proyecto ágil y eficiente, filtramos lo que realmente aporta valor de lo que resulta sobre-ingeniería innecesaria:

### ❌ Prácticas OVERKILL para 1-2 personas en etapa temprana:

1. **Git Flow Tradicional:**
   * Mantener ramas concurrentes `master`, `develop`, `release-1.x`, `hotfix-2.x`.
   * Añade burocracia innecesaria. Para un equipo pequeño no hay versiones en soporte legado simultáneo que justifiquen la fricción de sincronizar 4 ramas.
2. **Husky / Commitlint basado en Node.js:**
   * Instalar herramientas de Node (`npm install husky @commitlint/cli`) para validar mensajes en githooks locales.
   * Meter dependencias de Node.js (`node_modules`) en un repositorio de **Rust puro** ensucia el entorno. Es preferible validar la sintaxis y calidad en GitHub Actions CI.
3. **Branch naming con Jira Issue IDs:**
   * Prefijar ramas como `1110-feature/xyz`.
   * Innecesario cuando no existe un Jira corporativo; `feat/ast-parser` es más conciso y claro.

###  Prácticas de ALTO VALOR INMEDIATO:

1. **Tronco único (`main`) siempre verde:**
   * Migración definitiva a `main` y validación automatizada mediante CI para evitar roturas.
2. **Commits semánticos, atómicos e imperativos:**
   * Mensajes concisos en imperativo (`feat: add ...`, `fix: ...`) que cuentan la historia del proyecto y permiten generar changelogs automáticos.
3. **Pull Requests pequeñas y enfocadas:**
   * PRs atómicas que abordan un solo problema (evitando mezclar refactors de formato con nueva lógica).
4. **Higiene estricta de `.gitignore`:**
   * Prohibido commitear archivos de build (`/target`), bases de datos SQLite locales (`*.db`), índices de Tantivy y variables de entorno (`.env`).

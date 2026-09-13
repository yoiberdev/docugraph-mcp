# Contribuir a DocuGraph MCP

¡Gracias por tu interés en contribuir a **DocuGraph MCP**!

Para mantener el proyecto ágil, predecible y con alta calidad de ingeniería, seguimos un flujo de trabajo optimizado para equipos pequeños y código abierto.

---

## 🚀 Flujo de Trabajo (Branching Model)

Seguimos un modelo **Simplified Trunk-Based** con paradigma *Ship / Show / Ask*:

* La rama principal es `main` y debe mantenerse siempre estable y testeada.
* Para cambios significativos, crea ramas efímeras (1-2 días):
  ```bash
  git switch -c feat/nombre-caracteristica main
  # o
  git switch -c fix/nombre-del-bug main
  ```
* Consulta la especificación detallada en [`docs/git-workflow.md`](file:///docs/git-workflow.md).

---

## ✍️ Convención de Commits

Seguimos **Conventional Commits**:
```text
<tipo>: <descripción en imperativo>
```

Tipos principales:
* `feat:` Nueva funcionalidad para el usuario o agente.
* `fix:` Corrección de bug.
* `refactor:` Reorganización de código interno sin cambios funcionales.
* `perf:` Mejoras de rendimiento o reducción de uso de memoria.
* `docs:` Cambios o mejoras en la documentación.
* `test:` Adición o mejora de tests unitarios/integración.

---

## 🧪 Estándar de Calidad Local

Antes de enviar una Pull Request o hacer merge, verifica que se cumplan las siguientes condiciones:

```bash
# 1. Formato de código idiomático
cargo fmt --all -- --check

# 2. Linter estricto de Rust sin advertencias
cargo clippy --all-targets -- -D warnings

# 3. Suite completa de pruebas pasando
cargo test --all
```

---

## 🔒 Reglas Críticas del Servidor MCP
1. **`stdout` limpio:** Todo logging debe realizarse mediante `tracing` hacia `stderr`. Nunca uses `println!` para depuración en rutas de producción.
2. **Sin secretos:** Nunca hagas commit de archivos `.env` ni claves de API.
3. **Sin PDFs protegidos:** Nunca subas PDFs de libros comerciales con derechos de autor al repositorio.

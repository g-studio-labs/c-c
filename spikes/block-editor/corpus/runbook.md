# Runbook de despliegue

El backend despliega **antes** que la app. Al revés, una app nueva habla con un
servidor que no la entiende, y quien actualiza rápido *se come el rato*.

## Antes de empezar

1. Comprueba que `main` está en verde.
2. Mira el [tablero de despliegues](https://example.com/deploys "Los últimos diez") y el [runbook viejo][viejo].
3. Avisa en el canal.

Lo que **no** se hace nunca:

- Re-cortar una versión publicada.
- Desplegar un viernes por la tarde.
- Saltarse la firma (`--no-gpg-sign`).

### Variables

| Variable      | Para qué                | Por defecto |
|:--------------|:-----------------------:|------------:|
| `DB_HOST`     | La base de pruebas      | *(ninguna)* |
| `SITE_URL`    | El dominio canónico     | `localhost` |
| `LOG \| TEE`  | Una barra escapada      | —           |

```sh
(cd backend && go build ./... && go vet ./... && go test ./...)
# una línea en blanco dentro del código:

echo "hecho"
```

Una línea con salto duro al final (dos espacios)  
y otra con barra invertida\
y la última, que además sigue en la línea de abajo
sin más (salto blando).

![Diagrama del flujo](img/flujo.png "El flujo, de izquierda a derecha")

> **Ojo:** un egress muerto sigue diciendo que está vivo.
>
> - Sólo se pregunta al cerrar.
> - Contar el 412 como muerte tira grabaciones buenas.

<details>
<summary>La salida completa</summary>

<pre>
  línea   con   espacios
</pre>

</details>

- [x] Backend desplegado
- [ ] App publicada
- [ ] `STATUS.md` al día

Una nota al pie[^egress] y otra más[^1].

[^egress]: Medido contra el SFU, con los números en `docs/grabacion.md`.
[^1]: La corta.

[viejo]: https://example.com/runbook-2025 'El de antes'

* * *

Señal de vida: ¿ñandú, pingüino, cigüeña? ¡Sí! — «comillas», 日本語, 🚀.

# types

the vector package record types

## Synopsis

```
vector::Float2
vector::Float3
vector::Float4
vector::Fixed2
vector::Fixed3
vector::Fixed4
vector::Integer2
vector::Integer3
vector::Integer4
```

## Package

vector

## Imports

```
IMPORT vector
```

`vector` is a built-in package, so `IMPORT vector` needs no manifest
dependency.

## Description

The `vector` package provides nine fixed-width math-vector value records, one
per element type (`Float`, `Fixed`, `Integer`) and dimension (2, 3, 4). Each is
an ordinary value record of N homogeneous 8-byte fields named `x`, `y`, `z`,
`w`, in that order, as many as the dimension: a 2-vector has `x` and `y`, a
3-vector adds `z`, a 4-vector adds `w`. The element type of every component
matches the type's name — `Float` components are IEEE doubles, `Fixed`
components are Q32.32, and `Integer` components are 64-bit signed integers.

These records copy by value, drop with no heap frees (every component is a
scalar), and are thread-sendable. Construct one positionally with bracket
syntax, one argument per field of the element type — `vector::Float3[3.0, 0.0,
4.0]`, `vector::Integer2[1, 2]`, `vector::Fixed4[toFixed(1.0), toFixed(0.0),
toFixed(0.0), toFixed(0.0)]`. Read a component with field access (`v.x`, `v.y`,
`v.z`, `v.w`). The functions in this package are overloaded on these types; see
`mfb man vector`.

## Types

### vector::Float2

A 2D vector with `Float` (IEEE double) components.

| Field | Type | Description |
| --- | --- | --- |
| `x` | `Float` | first axis component |
| `y` | `Float` | second axis component |

### vector::Float3

A 3D vector with `Float` components (adds `z` to `Float2`).

| Field | Type | Description |
| --- | --- | --- |
| `x` | `Float` | first axis component |
| `y` | `Float` | second axis component |
| `z` | `Float` | third axis component |

### vector::Float4

A 4D vector with `Float` components (adds `w` to `Float3`).

| Field | Type | Description |
| --- | --- | --- |
| `x` | `Float` | first axis component |
| `y` | `Float` | second axis component |
| `z` | `Float` | third axis component |
| `w` | `Float` | fourth axis component |

### vector::Fixed2

A 2D vector with `Fixed` (Q32.32) components.

| Field | Type | Description |
| --- | --- | --- |
| `x` | `Fixed` | first axis component |
| `y` | `Fixed` | second axis component |

### vector::Fixed3

A 3D vector with `Fixed` components (adds `z` to `Fixed2`).

| Field | Type | Description |
| --- | --- | --- |
| `x` | `Fixed` | first axis component |
| `y` | `Fixed` | second axis component |
| `z` | `Fixed` | third axis component |

### vector::Fixed4

A 4D vector with `Fixed` components (adds `w` to `Fixed3`).

| Field | Type | Description |
| --- | --- | --- |
| `x` | `Fixed` | first axis component |
| `y` | `Fixed` | second axis component |
| `z` | `Fixed` | third axis component |
| `w` | `Fixed` | fourth axis component |

### vector::Integer2

A 2D vector with `Integer` (64-bit signed) components.

| Field | Type | Description |
| --- | --- | --- |
| `x` | `Integer` | first axis component |
| `y` | `Integer` | second axis component |

### vector::Integer3

A 3D vector with `Integer` components (adds `z` to `Integer2`).

| Field | Type | Description |
| --- | --- | --- |
| `x` | `Integer` | first axis component |
| `y` | `Integer` | second axis component |
| `z` | `Integer` | third axis component |

### vector::Integer4

A 4D vector with `Integer` components (adds `w` to `Integer3`).

| Field | Type | Description |
| --- | --- | --- |
| `x` | `Integer` | first axis component |
| `y` | `Integer` | second axis component |
| `z` | `Integer` | third axis component |
| `w` | `Integer` | fourth axis component |

## See also

- `mfb man vector`
- `mfb man vector length`
- `mfb man vector normalize`
- `mfb man vector dot`
- `mfb man vector cross`
- `mfb man vector lerp`

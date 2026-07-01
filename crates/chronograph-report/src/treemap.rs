//! Детерминированный squarified treemap (Bruls et al.).
//!
//! Раскладка — чистая функция упорядоченного списка площадей: вход сортируется
//! вызывающим по `(area desc, path asc)` ДО раскладки, поэтому при равных площадях
//! порядок фиксирован и вывод детерминирован (не зависит от итерации map'ов).

/// Прямоугольник раскладки: координаты + индекс исходного элемента.
#[derive(Debug, Clone, PartialEq)]
pub struct TreemapRect {
    /// Индекс элемента во входном (упорядоченном) списке.
    pub index: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Разложить площади `areas` (в порядке убывания) в прямоугольник `width×height`.
///
/// Площади масштабируются так, что их сумма = площади контейнера; каждый
/// прямоугольник сохраняет пропорцию исходной величины (ключевое свойство
/// treemap). Возврат — по одному прямоугольнику на элемент, в исходном порядке.
pub fn squarify(areas: &[f64], width: f64, height: f64) -> Vec<TreemapRect> {
    let n = areas.len();
    let mut out: Vec<TreemapRect> = Vec::with_capacity(n);
    if n == 0 || width <= 0.0 || height <= 0.0 {
        return out;
    }
    let total: f64 = areas.iter().copied().filter(|a| *a > 0.0).sum();
    if total <= 0.0 {
        return out;
    }
    let scale = (width * height) / total;
    // Масштабированные площади с исходными индексами.
    let scaled: Vec<(usize, f64)> = areas
        .iter()
        .enumerate()
        .map(|(i, a)| (i, a.max(0.0) * scale))
        .collect();

    // Свободный прямоугольник.
    let mut rx = 0.0_f64;
    let mut ry = 0.0_f64;
    let mut rw = width;
    let mut rh = height;

    let mut i = 0usize;
    let mut row: Vec<(usize, f64)> = Vec::new();
    while i < n {
        let side = rw.min(rh);
        let item = scaled[i];
        if row.is_empty() {
            row.push(item);
            i += 1;
            continue;
        }
        let cur_worst = worst_ratio(&row, side);
        let mut trial = row.clone();
        trial.push(item);
        let new_worst = worst_ratio(&trial, side);
        if new_worst <= cur_worst {
            row.push(item);
            i += 1;
        } else {
            // Зафиксировать ряд и уменьшить свободный прямоугольник.
            layout_row(&row, &mut rx, &mut ry, &mut rw, &mut rh, &mut out);
            row.clear();
        }
    }
    if !row.is_empty() {
        layout_row(&row, &mut rx, &mut ry, &mut rw, &mut rh, &mut out);
    }
    out
}

/// Худшая (максимальная) пропорция сторон прямоугольников ряда, уложенного вдоль
/// стороны длиной `side`.
fn worst_ratio(row: &[(usize, f64)], side: f64) -> f64 {
    let sum: f64 = row.iter().map(|(_, a)| *a).sum();
    if sum <= 0.0 || side <= 0.0 {
        return f64::INFINITY;
    }
    let mut max = f64::MIN;
    let mut min = f64::MAX;
    for (_, a) in row {
        max = max.max(*a);
        min = min.min(*a);
    }
    let s2 = sum * sum;
    let side2 = side * side;
    let r1 = (side2 * max) / s2;
    let r2 = s2 / (side2 * min);
    r1.max(r2)
}

/// Уложить ряд вдоль более короткой стороны свободного прямоугольника и сдвинуть
/// его границы.
fn layout_row(
    row: &[(usize, f64)],
    rx: &mut f64,
    ry: &mut f64,
    rw: &mut f64,
    rh: &mut f64,
    out: &mut Vec<TreemapRect>,
) {
    let sum: f64 = row.iter().map(|(_, a)| *a).sum();
    if sum <= 0.0 {
        return;
    }
    if *rw >= *rh {
        // Ряд — вертикальная колонка шириной `col_w` слева.
        let col_w = sum / *rh;
        let mut y = *ry;
        for (idx, a) in row {
            let h = a / col_w;
            out.push(TreemapRect {
                index: *idx,
                x: *rx,
                y,
                w: col_w,
                h,
            });
            y += h;
        }
        *rx += col_w;
        *rw -= col_w;
    } else {
        // Ряд — горизонтальная полоса высотой `row_h` сверху.
        let row_h = sum / *rw;
        let mut x = *rx;
        for (idx, a) in row {
            let w = a / row_h;
            out.push(TreemapRect {
                index: *idx,
                x,
                y: *ry,
                w,
                h: row_h,
            });
            x += w;
        }
        *ry += row_h;
        *rh -= row_h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_area_proportions() {
        // Ключевое свойство treemap: площадь прямоугольника ∝ входной величине.
        let areas = vec![50.0, 30.0, 20.0, 5.0, 5.0];
        let (w, h) = (400.0, 300.0);
        let rects = squarify(&areas, w, h);
        assert_eq!(rects.len(), areas.len());
        let total: f64 = areas.iter().sum();
        let scale = (w * h) / total;
        for r in &rects {
            let expected = areas[r.index] * scale;
            let got = r.w * r.h;
            assert!(
                (got - expected).abs() < 1e-6,
                "площадь index {} ожидалась {expected}, получено {got}",
                r.index
            );
        }
    }

    #[test]
    fn rects_stay_within_container() {
        let areas = vec![10.0, 7.0, 3.0, 2.0, 1.0];
        let (w, h) = (500.0, 200.0);
        for r in squarify(&areas, w, h) {
            assert!(r.x >= -1e-6 && r.y >= -1e-6);
            assert!(r.x + r.w <= w + 1e-6);
            assert!(r.y + r.h <= h + 1e-6);
        }
    }

    #[test]
    fn deterministic_for_equal_areas() {
        // Равные площади — раскладка воспроизводима (детерминизм).
        let areas = vec![1.0, 1.0, 1.0, 1.0];
        let a = squarify(&areas, 300.0, 300.0);
        let b = squarify(&areas, 300.0, 300.0);
        assert_eq!(a, b);
    }

    #[test]
    fn empty_input_yields_no_rects() {
        assert!(squarify(&[], 100.0, 100.0).is_empty());
    }
}

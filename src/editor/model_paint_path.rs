//! Edit a surface or UV curve and submit it through the same GPU queue as the brush.
use super::*;
use crate::editor::area_selection;

const POINT_RADIUS: f32 = 9.;
const SUBDIVISIONS: usize = 32;
const PREVIEW_STAMP_LIMIT: usize = 600;

#[derive(Clone, Copy)]
struct PathSample {
    sample: Sample,
    view: Option<gpu::View>,
}

#[derive(Default)]
pub(super) struct EditPath {
    uv: Option<bool>,
    points: Vec<Vec3>,
    dragging: Option<usize>,
    right_start: Option<Vec2>,
    pending: VecDeque<PathSample>,
    applied: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Views {
    pub model: Option<CameraMap>,
    pub uv: Option<CameraMap>,
}

impl Views {
    pub fn view(self, uv: bool) -> Option<gpu::View> {
        Some(gpu::View {
            camera: if uv { self.uv? } else { self.model? },
            uv,
        })
    }

    fn at(self, point: Vec2) -> Option<gpu::View> {
        [true, false]
            .into_iter()
            .filter_map(|uv| self.view(uv))
            .find(|view| view.camera.clip.contains(point))
    }
}

pub(super) struct Input<'a> {
    pub doc: &'a Document,
    pub object: &'a Object,
    pub visibility: &'a sets::SetVisibility,
    pub slot: usize,
    pub views: Views,
    pub samples: &'a [Sample],
    pub cursor: Option<Vec2>,
    pub keys: &'a ButtonInput<KeyCode>,
    pub mouse: &'a ButtonInput<MouseButton>,
    pub can_start: &'a dyn Fn(Vec2) -> bool,
}

pub(super) enum Outcome {
    Idle,
    Applied,
    Status(&'static str),
}

#[derive(Default)]
pub(super) struct Overlay {
    pub lines: Vec<[Vec2; 2]>,
    pub circles: Vec<(Vec2, f32)>,
    pub points: Vec<ControlPoint>,
}

pub(super) struct ControlPoint {
    pub center: Vec2,
    pub radius: f32,
    pub first: bool,
}

impl EditPath {
    pub fn cancel_pointer(&mut self) {
        self.dragging = None;
        self.right_start = None;
        self.pending.clear();
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    fn nearest(&self, view: gpu::View, point: Vec2) -> Option<usize> {
        if self.uv != Some(view.uv) {
            return None;
        }
        self.points
            .iter()
            .enumerate()
            .filter_map(|(i, &p)| Some((i, project(view, p)?.distance_squared(point))))
            .filter(|&(_, distance)| distance <= POINT_RADIUS * POINT_RADIUS)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    fn remove(&mut self, index: usize) {
        self.points.remove(index);
        self.dragging = None;
        self.applied = false;
        if self.points.is_empty() {
            self.uv = None;
        }
    }

    fn pointer(&mut self, input: &Input, trace: Option<&surface::SurfaceTrace>) {
        if input.mouse.just_pressed(MouseButton::Right) {
            self.dragging = None;
            self.right_start = input.cursor.filter(|&p| (input.can_start)(p));
        }
        if self
            .right_start
            .zip(input.cursor)
            .is_some_and(|(start, point)| area_selection::dragged(start, point))
        {
            self.right_start = None;
        }
        if input.mouse.just_released(MouseButton::Right)
            && let Some(point) = self.right_start.take()
            && let Some(view) = input.views.at(point)
            && let Some(i) = self.nearest(view, point)
        {
            self.remove(i);
        }
        if input.mouse.pressed(MouseButton::Right) {
            return;
        }

        for &sample in input.samples {
            let point = match sample {
                Sample::Down(point) if (input.can_start)(point) => point,
                Sample::Move(point) | Sample::Up(point) => point,
                _ => continue,
            };
            if matches!(sample, Sample::Move(_))
                && self
                    .pending
                    .back()
                    .is_some_and(|p| matches!(p.sample, Sample::Move(_)))
            {
                self.pending.pop_back();
            }
            self.pending.push_back(PathSample {
                sample,
                view: input.views.at(point),
            });
        }
        while let Some(event) = self.pending.pop_front() {
            if let Sample::Up(_) = event.sample {
                self.dragging = None;
                continue;
            }
            let Some(view) = event.view else { continue };
            let point = match event.sample {
                Sample::Down(point) => {
                    self.dragging = self.nearest(view, point);
                    if self.dragging.is_some() {
                        continue;
                    }
                    point
                }
                Sample::Move(point) if self.dragging.is_some() && self.uv == Some(view.uv) => point,
                _ => continue,
            };
            let placed = if view.uv {
                Some(((point - view.camera.rect.min) / view.camera.rect.size()).extend(0.))
            } else {
                let Some(trace) = trace else {
                    self.pending.push_front(event);
                    break;
                };
                trace
                    .pick_surface(view.camera, point, input.object.id, input.slot)
                    .map(|(_, hit)| hit)
            };
            if let Some(point) = placed {
                if let Some(index) = self.dragging {
                    self.points[index] = point;
                } else {
                    if self.uv != Some(view.uv) {
                        self.points.clear();
                        self.uv = Some(view.uv);
                    }
                    self.points.push(point);
                    self.dragging = Some(self.points.len() - 1);
                }
                self.applied = false;
            }
        }
    }

    pub fn overlay(&self, views: Views, session: &Session, cursor: Option<Vec2>) -> Overlay {
        let Some(view) = self.uv.and_then(|uv| views.view(uv)) else {
            return Overlay::default();
        };
        let rect = view.camera.clip;
        let radius = session.size * 0.5;
        let mut lines = if self.applied {
            Vec::new()
        } else {
            segments(&self.points, view, session.path_smooth, session.path_closed)
        };
        let mut circles = Vec::new();
        if !lines.is_empty() {
            let step = (session.size * session.spacing / 100.).max(1.);
            let sample_area = if session.path_align {
                rect.inflate(radius * 0.8)
            } else {
                rect.inflate(-radius)
            };
            for stamp in resample(&lines, step, session.path_closed, Some(sample_area)) {
                if rect.contains(stamp.point - Vec2::splat(radius))
                    && rect.contains(stamp.point + Vec2::splat(radius))
                {
                    circles.push((stamp.point, radius));
                }
                if session.path_align
                    && let Some(heading) = stamp.heading
                {
                    let direction =
                        Vec2::from_angle(heading + session.tip_angle.to_radians()) * radius * 0.8;
                    lines.extend(area_selection::clip_segment(
                        rect,
                        stamp.point - direction,
                        stamp.point + direction,
                    ));
                }
            }
        }
        let hovered = self.dragging.or_else(|| {
            let cursor = cursor.filter(|&point| rect.contains(point))?;
            self.nearest(view, cursor)
        });
        let points = self
            .points
            .iter()
            .enumerate()
            .filter_map(|(i, &point)| {
                let center = project(view, point)?;
                let radius = if hovered == Some(i) { 6.5 } else { 5. };
                rect.inflate(-radius).contains(center).then_some(ControlPoint {
                    center,
                    radius,
                    first: i == 0,
                })
            })
            .collect();
        lines = lines.into_iter().filter_map(|[a, b]| area_selection::clip_segment(rect, a, b)).collect();
        Overlay {
            lines,
            circles,
            points,
        }
    }
}

pub(super) fn edit(
    live: &mut Live,
    session: &mut Session,
    tips: &tips::Library,
    input: Input,
) -> Outcome {
    let command = session.path_command.take();
    if input.keys.just_pressed(KeyCode::Escape) || command == Some(PaintAction::PathClear) {
        live.path.clear();
    } else {
        if input.keys.just_pressed(KeyCode::Backspace) && !live.path.points.is_empty() {
            live.path.remove(live.path.points.len() - 1);
        }
        let trace = input.views.model.and_then(|_| {
            live.controller
                .trace(input.doc.revision, input.visibility, &input.doc.objects, false)
        });
        live.path.pointer(&input, trace.as_deref());
    }
    session.path_count = live.path.points.len();
    if !input.keys.just_pressed(KeyCode::Enter) && command != Some(PaintAction::PathApply) {
        return Outcome::Idle;
    }
    if live.path.points.len() < 2 {
        return Outcome::Status("Поставьте хотя бы две точки контура.");
    }
    if !session.can_paint_layer() || !has_uv(input.object, input.slot) {
        return Outcome::Status("Для контура нужен видимый слой краски и UV-развёртка.");
    }
    let Some(view) = live.path.uv.and_then(|uv| input.views.view(uv)) else {
        return Outcome::Status("Откройте вид, в котором создан контур.");
    };
    let line = segments(
        &live.path.points,
        view,
        session.path_smooth,
        session.path_closed,
    );
    let stamps = resample(
        &line,
        (session.size * session.spacing / 100.).max(1.),
        session.path_closed,
        None,
    );
    if stamps.is_empty() {
        return Outcome::Status("Переместите контур в видимую часть окна.");
    }
    let mut brush = brush_for(session, tips, input.object, view.uv, false);
    if let Some(tip) = &mut brush.tip {
        tip.follow |= session.path_align;
    }
    let map = Mapping::for_camera(input.doc, input.object, input.slot, view.camera, view.uv, input.visibility);
    live.queue.push_back(Event::Begin(brush));
    live.enqueue_stamps(stamps, Arc::new(map));
    live.queue.push_back(Event::End);
    live.path.applied = true;
    Outcome::Applied
}

fn project(view: gpu::View, point: Vec3) -> Option<Vec2> {
    if view.uv {
        Some(view.camera.rect.min + point.truncate() * view.camera.rect.size())
    } else {
        view.camera.project(point)
    }
}

fn segments(points: &[Vec3], view: gpu::View, smooth: bool, closed: bool) -> Vec<[Vec2; 2]> {
    curve(points, smooth, closed)
        .windows(2)
        .filter_map(|pair| {
            area_selection::clip_segment(
                view.camera.rect,
                project(view, pair[0])?,
                project(view, pair[1])?,
            )
        })
        .collect()
}

fn curve(points: &[Vec3], smooth: bool, closed: bool) -> Vec<Vec3> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let count = points.len() as isize;
    let segments = if closed { count } else { count - 1 };
    let at = |i: isize| {
        points[if closed {
            i.rem_euclid(count)
        } else {
            i.clamp(0, count - 1)
        } as usize]
    };
    let mut line =
        Vec::with_capacity(segments as usize * if smooth { SUBDIVISIONS } else { 1 } + 1);
    for i in 0..segments {
        line.push(at(i));
        if smooth {
            let points = [at(i - 1), at(i), at(i + 1), at(i + 2)];
            for step in 1..SUBDIVISIONS {
                line.push(catmull_rom(points, step as f32 / SUBDIVISIONS as f32));
            }
        }
    }
    line.push(at(segments));
    line
}

fn catmull_rom([a, b, c, d]: [Vec3; 4], t: f32) -> Vec3 {
    let knot = |a: Vec3, b: Vec3| a.distance(b).sqrt().max(1e-5);
    let t1 = knot(a, b);
    let t2 = t1 + knot(b, c);
    let t3 = t2 + knot(c, d);
    let u = t1 + (t2 - t1) * t;
    let interpolate = |a: Vec3, b: Vec3, ta, tb| a.lerp(b, (u - ta) / (tb - ta));
    let ab = interpolate(a, b, 0., t1);
    let bc = interpolate(b, c, t1, t2);
    let cd = interpolate(c, d, t2, t3);
    interpolate(
        interpolate(ab, bc, 0., t2),
        interpolate(bc, cd, t1, t3),
        t1,
        t2,
    )
}

fn resample(segments: &[[Vec2; 2]], step: f32, closed: bool, preview: Option<Rect>) -> Vec<Stamp> {
    let mut stamps = Vec::new();
    let mut carry = 0.;
    let mut previous_end = None;
    let mut first_stamp = None;
    let mut last_stamp = None;
    for &[a, b] in segments {
        let displacement = b - a;
        let length = displacement.length();
        if length < 1e-5 {
            continue;
        }
        let direction = displacement / length;
        let heading = Some(direction.to_angle());
        let starts_run = previous_end.is_none_or(|point: Vec2| point.distance_squared(a) > 1e-4);
        if starts_run {
            first_stamp.get_or_insert(a);
            last_stamp = Some(a);
            carry = 0.;
        }
        let start = step - carry;
        let count = (((length - start) / step).floor() + 1.) as usize;
        let distance = |index: usize| start + index as f32 * step;
        if count > 0 {
            last_stamp = Some(a + direction * distance(count - 1));
            carry = length - distance(count - 1);
        } else {
            carry += length;
        }
        previous_end = Some(b);

        // Skip offscreen samples without changing the spacing phase of the visible samples.
        let indices = if let Some(rect) = preview {
            let Some([near, far]) = area_selection::clip_segment(rect, a, b) else { continue };
            let index = |point: Vec2| ((point - a).dot(direction) - start) / step;
            let first = (index(near).floor() as usize).min(count);
            let last = (index(far).ceil() as usize).saturating_add(1).min(count);
            first..last
        } else {
            0..count
        };
        let run_start = starts_run.then_some(a);
        for point in run_start.into_iter().chain(indices.map(|i| a + direction * distance(i))) {
            if let Some(rect) = preview {
                if !rect.contains(point) {
                    continue;
                }
                if stamps.len() == PREVIEW_STAMP_LIMIT {
                    return stamps;
                }
            }
            stamps.push(Stamp {
                heading,
                ..Stamp::from(point)
            });
        }
    }
    if closed
        && let Some((first, last)) = first_stamp.zip(last_stamp)
        && first.distance(last) < step * 0.5
        && stamps.last().is_some_and(|stamp| stamp.point == last)
        && (stamps.len() > 1 || last != first)
    {
        stamps.pop();
    }
    stamps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_path_curve_contains_the_control_points_and_closes() {
        let points = [Vec3::ZERO, Vec3::X, Vec3::new(2., 8., 0.)];
        let line = curve(&points, true, true);
        assert_eq!(line.first(), line.last());
        assert_eq!(line[32], points[1]);
        assert_eq!(line[64], points[2]);
        assert!(line.iter().all(|p| p.is_finite()));
        assert_eq!(curve(&points, false, false), points);
    }

    #[test]
    fn paint_path_resampling_keeps_spacing_and_heading_at_corners() {
        let horizontal = [Vec2::ZERO, Vec2::X * 5.];
        let vertical = [horizontal[1], Vec2::new(5., 5.)];
        let stamps = resample(&[horizontal, vertical], 3., false, None);
        let positions: Vec<_> = stamps.iter().map(|stamp| stamp.point).collect();
        assert_eq!(
            positions,
            [
                Vec2::ZERO,
                Vec2::new(3., 0.),
                Vec2::new(5., 1.),
                Vec2::new(5., 4.)
            ]
        );
        assert_eq!(stamps[0].heading, Some(0.));
        assert_eq!(stamps[2].heading, Some(std::f32::consts::FRAC_PI_2));
        let loop_stamps = resample(&[horizontal, [horizontal[1], horizontal[0]]], 2., true, None);
        assert_eq!(loop_stamps.len(), 5);
    }

    #[test]
    fn paint_path_preview_skips_offscreen_stamps_and_preserves_spacing() {
        let segments = [
            [Vec2::ZERO, Vec2::new(9000., 0.)],
            [Vec2::new(9000., 0.), Vec2::new(9000., 5000.)],
        ];
        let clip = Rect::new(8900., -1., 9100., 4000.);
        let applied = resample(&segments, 3.5, false, None);
        let expected: Vec<_> = applied
            .into_iter()
            .filter(|stamp| clip.contains(stamp.point))
            .take(PREVIEW_STAMP_LIMIT)
            .collect();
        assert_eq!(expected.len(), PREVIEW_STAMP_LIMIT);
        assert_eq!(resample(&segments, 3.5, false, Some(clip)), expected);
    }
}

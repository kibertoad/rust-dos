//! Drawing commands: a triangle from the parameters the host wrote (its
//! texture units, the subpixel adjustment, the buffer it goes into, and
//! its scan conversion into scanlines), and the fastfill that clears a
//! rectangle. From DOSBox-X's `triangle`, `triangle_create_work_item`,
//! `poly_render_triangle` and `fastfill` (voodoo_emu.cpp).

use super::raster::{RasterState, TmuParams, TmuRaster, TriParams};
use super::backlog;
use super::workers::Job;
use super::regs::*;
use super::{NONE, Voodoo};

pub(crate) mod scan;
pub(crate) use scan::render_triangle;

impl Voodoo {
    /// The word offset of a colour buffer, or None for a reserved choice.
    fn color_buffer(&self, select: u32) -> Option<usize> {
        let index = match select {
            // A Banshee draws where colBufferAddr says, whatever fbzMode
            // selects.
            _ if self.banshee.is_some() => 0,
            0 => self.fbi.frontbuf,
            1 => self.fbi.backbuf,
            _ => return None,
        };
        let offs = self.fbi.rgboffs[index as usize];
        (offs != NONE).then_some(offs as usize / 2)
    }

    /// What drawing into colour buffer `dest` reads from the registers.
    pub(crate) fn raster_state(&self, dest: usize, texcount: usize) -> RasterState {
        let reg = &self.reg;
        let chip = self.chip;
        let tmu = |unit: usize| -> Option<TmuRaster> {
            if unit >= texcount {
                return None;
            }
            let t = &self.tmu[unit];
            let unfiltered = self.texture_sampling == super::TextureSampling::Unfiltered;
            let mode = reg[t.base + TEXTURE_MODE] & if unfiltered { !6 } else { !0 };
            Some(TmuRaster {
                ram: t.ram.clone(),
                mask: t.mask,
                mode,
                unfiltered,
                lodmin: t.lodmin,
                lodmax: t.lodmax,
                lodbias: t.lodbias,
                lodmask: t.lodmask,
                lodoffset: t.lodoffset,
                detailmax: t.detailmax,
                detailbias: t.detailbias,
                detailscale: t.detailscale,
                wmask: t.wmask as i32,
                hmask: t.hmask as i32,
                lookup: t.lookup(mode),
                bilinear_mask: chip.bilinear_mask,
            })
        };
        RasterState {
            fb: self.fbi.ram.clone(),
            fbz_color_path: reg[FBZ_COLOR_PATH],
            fbz_mode: reg[FBZ_MODE],
            alpha_mode: reg[ALPHA_MODE],
            fog_mode: reg[FOG_MODE],
            za_color: reg[ZA_COLOR],
            chroma_key: reg[CHROMA_KEY],
            color0: reg[COLOR0],
            color1: reg[COLOR1],
            fog_color: reg[FOG_COLOR],
            clip_left_right: reg[CLIP_LEFT_RIGHT],
            clip_low_y_high_y: reg[CLIP_LOW_Y_HIGH_Y],
            clip_mask: chip.clip_mask,
            // A Voodoo Graphics hasn't the register: always 0 there.
            chroma_range: reg[CHROMA_RANGE],
            fogdelta_mask: chip.fogdelta_mask,
            yorigin: self.fbi.yorigin,
            ymask: self.fbi.ymask,
            rowpixels: self.fbi.rowpixels as usize,
            stride: self.fbi.stride,
            aux_stride: self.fbi.aux_stride,
            dest,
            aux: (self.fbi.auxoffs != NONE).then_some(self.fbi.auxoffs as usize / 2),
            fogblend: self.fbi.fogblend,
            fogdelta: self.fbi.fogdelta,
            send_config: self.send_config.then_some(self.tmu_config),
            stipple: reg[STIPPLE],
            tmu: [tmu(0), tmu(1)],
        }
    }

    /// triangleCMD: draw the triangle the registers describe.
    pub(crate) fn triangle(&mut self) {
        let mut texcount = 0;
        if self.reg[FBI_INIT3] & (1 << 6) == 0 && self.reg[FBZ_COLOR_PATH] & (1 << 27) != 0 {
            texcount = self.tmu.len();
        }

        // Subpixel adjustment: the start values move from vertex A to the
        // centre of its pixel.
        if self.reg[FBZ_COLOR_PATH] & (1 << 26) != 0 {
            let f = &mut self.fbi;
            let dx = 8 - (f.ax & 15) as i32;
            let dy = 8 - (f.ay & 15) as i32;
            let adj = |d_dy: i32, d_dx: i32| dy.wrapping_mul(d_dy).wrapping_add(dx.wrapping_mul(d_dx)) >> 4;
            let adj64 =
                |d_dy: i64, d_dx: i64| (dy as i64).wrapping_mul(d_dy).wrapping_add((dx as i64).wrapping_mul(d_dx)) >> 4;
            f.startr = f.startr.wrapping_add(adj(f.drdy, f.drdx));
            f.startg = f.startg.wrapping_add(adj(f.dgdy, f.dgdx));
            f.startb = f.startb.wrapping_add(adj(f.dbdy, f.dbdx));
            f.starta = f.starta.wrapping_add(adj(f.dady, f.dadx));
            f.startw = f.startw.wrapping_add(adj64(f.dwdy, f.dwdx));
            let mul = |a: i32, b: i32| ((a as i64 * b as i64) >> 4) as i32;
            f.startz = f.startz.wrapping_add(mul(dy, f.dzdy)).wrapping_add(mul(dx, f.dzdx));
            for t in self.tmu.iter_mut().take(texcount) {
                t.startw = t.startw.wrapping_add(adj64(t.dwdy, t.dwdx));
                t.starts = t.starts.wrapping_add(adj64(t.dsdy, t.dsdx));
                t.startt = t.startt.wrapping_add(adj64(t.dtdy, t.dtdx));
            }
        }

        let Some(dest) = self.color_buffer((self.reg[FBZ_MODE] >> 14) & 3) else { return };

        let f = &self.fbi;
        let mut p = TriParams {
            ax: f.ax,
            ay: f.ay,
            startr: f.startr,
            startg: f.startg,
            startb: f.startb,
            starta: f.starta,
            startz: f.startz,
            startw: f.startw,
            drdx: f.drdx,
            dgdx: f.dgdx,
            dbdx: f.dbdx,
            dadx: f.dadx,
            dzdx: f.dzdx,
            dwdx: f.dwdx,
            drdy: f.drdy,
            dgdy: f.dgdy,
            dbdy: f.dbdy,
            dady: f.dady,
            dzdy: f.dzdy,
            dwdy: f.dwdy,
            tmu: Default::default(),
        };
        let verts = [(f.ax, f.ay), (f.bx, f.by), (f.cx, f.cy)].map(|(x, y)| (x as f32 / 16.0, y as f32 / 16.0));
        for unit in 0..texcount {
            let t = &mut self.tmu[unit];
            let lodbase = t.prepare(&self.reg[..]);
            p.tmu[unit] = TmuParams {
                starts: t.starts,
                startt: t.startt,
                startw: t.startw,
                dsdx: t.dsdx,
                dtdx: t.dtdx,
                dwdx: t.dwdx,
                dsdy: t.dsdy,
                dtdy: t.dtdy,
                dwdy: t.dwdy,
                lodbase,
            };
        }
        let st = self.raster_state(dest, texcount);
        if self.mirror.is_some() {
            self.mirror_sync();
            if let Some(mirror) = &mut self.mirror {
                mirror.triangle(&st, &p, verts, texcount, &self.tmu, self.reg[STIPPLE]);
            }
        }
        let access = self.access(|layout| backlog::triangle(layout, &st, &verts));
        let rotate = self.reg[FBZ_MODE] & (1 << 2 | 1 << 12) == 1 << 2;
        let spills = backlog::spills(&st, &verts, self.fbi.height);
        let job = Job::Triangle { st, p, verts, texcount };
        if rotate || spills {
            // Stippling in rotate mode goes on from pixel to pixel across
            // the triangle, and pixels right of the rows land in the next
            // row, which another worker draws: drawn here, whole.
            self.flush();
            let mut stipple = self.reg[STIPPLE];
            let mut stats = self.stats;
            job.run_all(&mut stipple, &mut stats);
            if rotate {
                self.reg[STIPPLE] = stipple;
            }
            self.stats = stats;
        } else {
            self.submit(job, access);
        }
        self.reg[FBI_TRIANGLES_OUT] = self.reg[FBI_TRIANGLES_OUT].wrapping_add(1);
        self.mark_drawn(dest);
    }

    /// Something was drawn into colour buffer `dest`: the display changes
    /// if it is the one shown (on a Banshee, if the desktop or the overlay
    /// shows its memory).
    pub(crate) fn mark_drawn(&mut self, dest: usize) {
        let shown = match &self.banshee {
            Some(b) => b.shows(dest as u32 * 2),
            None => self.color_buffer(0) == Some(dest),
        };
        if shown {
            self.display_dirty = true;
        }
        if !self.drawn_to.contains(&dest) {
            self.drawn_to.push(dest);
        }
    }

    /// fastfillCMD: fill the clip rectangle with color1 and the depth or
    /// alpha of zaColor, as fbzMode's masks say.
    pub(crate) fn fastfill(&mut self) {
        let fbz = self.reg[FBZ_MODE];
        let (rgb, aux) = (fbz & (1 << 9) != 0, fbz & (1 << 10) != 0);
        if !rgb && !aux {
            return;
        }
        let dest = if rgb { self.color_buffer((fbz >> 14) & 3) } else { Some(0) };
        let Some(dest) = dest else { return };
        let st = self.raster_state(dest, 0);
        let (sx, ex, sy, ey) = st.clip();
        // color1 through the dither matrix, 4x4 pixels.
        let mut dither = [0u16; 16];
        if rgb {
            let t = super::tables::tables();
            let c = self.reg[COLOR1];
            let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
            for y in 0..4usize {
                for x in 0..4usize {
                    let (r5, g6, b5) = if fbz & (1 << 8) != 0 {
                        let lookup = if fbz & (1 << 11) == 0 { &t.dither4 } else { &t.dither2 };
                        let at = y << 11 | x << 1;
                        (
                            lookup[at | (r as usize) << 3] as u16,
                            lookup[at | (g as usize) << 3 | 1] as u16,
                            lookup[at | (b as usize) << 3] as u16,
                        )
                    } else {
                        ((r >> 3) as u16, (g >> 2) as u16, (b >> 3) as u16)
                    };
                    dither[y * 4 + x] = r5 << 11 | g6 << 5 | b5;
                }
            }
        }
        if self.mirror.is_some() {
            self.mirror_sync();
            if let Some(mirror) = &mut self.mirror {
                mirror.fastfill(&st, (sx, ex, sy, ey), rgb, aux);
            }
        }
        let access = self.access(|layout| backlog::fastfill(layout, &st, (sx, ex, sy, ey)));
        let spills = backlog::fill_spills(&st, ex, sy, ey, self.fbi.height);
        let job = Job::Fastfill { st, dither, x0: sx, x1: ex, y0: sy, y1: ey };
        if spills {
            // Into the next rows' memory, which other workers draw.
            self.flush();
            let mut stats = self.stats;
            job.run_all(&mut 0, &mut stats);
            self.stats = stats;
        } else {
            self.submit(job, access);
        }
        if rgb {
            self.mark_drawn(dest);
        }
    }
}

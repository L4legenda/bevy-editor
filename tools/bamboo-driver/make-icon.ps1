param([Parameter(Mandatory=$true)][string]$Output)
$ErrorActionPreference = 'Stop'
Add-Type -ReferencedAssemblies System.Drawing @'
using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.IO;
public static class BambooIconBuilder {
    static GraphicsPath Round(float x, float y, float w, float h, float r) {
        GraphicsPath p = new GraphicsPath();
        p.AddArc(x,y,r*2,r*2,180,90); p.AddArc(x+w-r*2,y,r*2,r*2,270,90);
        p.AddArc(x+w-r*2,y+h-r*2,r*2,r*2,0,90); p.AddArc(x,y+h-r*2,r*2,r*2,90,90); p.CloseFigure(); return p;
    }
    public static void Build(string output) {
        int[] sizes = { 16, 20, 24, 32, 40, 48, 64, 128, 256 };
        byte[][] frames = new byte[sizes.Length][];
        for(int n=0;n<sizes.Length;n++) {
            using(Bitmap hi=new Bitmap(512,512)) {
                using(Graphics g=Graphics.FromImage(hi)) {
                    g.SmoothingMode=SmoothingMode.AntiAlias; g.ScaleTransform(8,8); g.Clear(Color.Transparent);
                    using(GraphicsPath p=Round(2,2,60,60,16))
                    using(LinearGradientBrush b=new LinearGradientBrush(new Point(4,2),new Point(60,62),Color.FromArgb(39,57,77),Color.FromArgb(16,25,35))) g.FillPath(b,p);
                    using(GraphicsPath p=Round(10,19,43,33,7)) {
                        using(SolidBrush b=new SolidBrush(Color.FromArgb(22,42,53))) g.FillPath(b,p);
                        using(LinearGradientBrush b=new LinearGradientBrush(new Point(10,19),new Point(53,52),Color.FromArgb(164,251,213),Color.FromArgb(75,213,176)))
                        using(Pen pen=new Pen(b,3)) g.DrawPath(pen,p);
                    }
                    using(Pen pen=new Pen(Color.FromArgb(95,144,241,206),2)) { pen.StartCap=pen.EndCap=LineCap.Round; g.DrawLine(pen,17,27,35,27); g.DrawLine(pen,17,32,28,32); }
                    using(GraphicsPath p=new GraphicsPath()) {
                        p.AddPolygon(new PointF[]{new PointF(29,43),new PointF(32,33),new PointF(48,13),new PointF(52,11),new PointF(56,15),new PointF(54,19),new PointF(38,39)});
                        using(Pen pen=new Pen(Color.FromArgb(17,29,40),6)) { pen.LineJoin=LineJoin.Round; g.DrawPath(pen,p); }
                        using(SolidBrush b=new SolidBrush(Color.FromArgb(255,218,151))) g.FillPath(b,p);
                    }
                    using(SolidBrush b=new SolidBrush(Color.FromArgb(247,249,237))) g.FillPolygon(b,new Point[]{new Point(32,33),new Point(29,43),new Point(38,39)});
                    using(SolidBrush b=new SolidBrush(Color.FromArgb(91,214,181))) g.FillPolygon(b,new Point[]{new Point(29,43),new Point(31,38),new Point(34,41)});
                    using(Pen pen=new Pen(Color.FromArgb(170,115,60),2)) g.DrawLine(pen,46,16,52,21);
                }
                using(Bitmap small=new Bitmap(sizes[n],sizes[n])) {
                    using(Graphics g=Graphics.FromImage(small)) { g.InterpolationMode=InterpolationMode.HighQualityBicubic; g.DrawImage(hi,0,0,sizes[n],sizes[n]); }
                    using(MemoryStream m=new MemoryStream()) { small.Save(m,System.Drawing.Imaging.ImageFormat.Png); frames[n]=m.ToArray(); }
                    if(sizes[n]==256) small.Save(Path.ChangeExtension(output,"png"),System.Drawing.Imaging.ImageFormat.Png);
                }
            }
        }
        using(BinaryWriter w=new BinaryWriter(File.Create(output))) {
            w.Write((ushort)0); w.Write((ushort)1); w.Write((ushort)sizes.Length);
            int offset=6+16*sizes.Length;
            for(int n=0;n<sizes.Length;n++) { w.Write((byte)(sizes[n]==256?0:sizes[n])); w.Write((byte)(sizes[n]==256?0:sizes[n])); w.Write((byte)0); w.Write((byte)0); w.Write((ushort)1); w.Write((ushort)32); w.Write(frames[n].Length); w.Write(offset); offset+=frames[n].Length; }
            foreach(byte[] frame in frames) w.Write(frame);
        }
    }
}
'@
[BambooIconBuilder]::Build([IO.Path]::GetFullPath($Output))

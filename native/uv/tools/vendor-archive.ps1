param([string]$RepositoryRoot = (Resolve-Path "$PSScriptRoot/../../..").Path)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.IO.Compression;
using System.Collections.Generic;
public static class UvHeaderArchive {
  static HashSet<string> seen = new HashSet<string>(StringComparer.Ordinal);
  static void Entry(ZipArchive output,string name,Stream source) {
    if(name.Contains("..") || name.StartsWith("/") || !seen.Add(name)) return;
    var entry=output.CreateEntry(name,CompressionLevel.Optimal);
    using(var target=entry.Open()) source.CopyTo(target);
  }
  static void Zip(ZipArchive output,string file,string kind) {
    using(var input=ZipFile.OpenRead(file)) foreach(var entry in input.Entries) {
      if(entry.Name.Length==0) continue;
      var full=entry.FullName;var first=full.IndexOf('/');
      if(first<0)continue;var relative=full.Substring(first+1);string name=null;
      if(kind=="eigen") {
        if(relative.StartsWith("Eigen/") || relative.StartsWith("unsupported/"))name=relative;
        else if(relative.StartsWith("COPYING"))name="licenses/eigen/"+relative;
      } else if(kind=="boost") {
        if(relative.StartsWith("boost/"))name=relative;
        else if(relative=="LICENSE_1_0.txt")name="licenses/boost/"+relative;
      } else if(kind=="igl") {
        if(relative.StartsWith("include/igl/"))name=relative.Substring(8);
        else if(relative.StartsWith("LICENSE"))name="licenses/libigl/"+relative;
      } else if(kind=="cgal") {
        var offset=relative.IndexOf("/include/CGAL/");
        if(offset>=0)name=relative.Substring(offset+9);
        else if(relative.StartsWith("LICENSE"))name="licenses/cgal/"+relative;
      }
      if(name!=null)using(var stream=entry.Open())Entry(output,name,stream);
    }
  }
  static void Folder(ZipArchive output,string root,string prefix) {
    foreach(var file in Directory.EnumerateFiles(root,"*",SearchOption.AllDirectories)) {
      var name=prefix+file.Substring(root.Length).TrimStart('\\','/').Replace('\\','/');
      using(var source=File.OpenRead(file))Entry(output,name,source);
    }
  }
  public static void Build(string root,string destination) {
    using(var output=new ZipArchive(File.Create(destination),ZipArchiveMode.Create)) {
      var sdk=Path.Combine(root,"target/partuv-native/deps");
      Zip(output,Path.Combine(sdk,"eigen.zip"),"eigen");
      Zip(output,Path.Combine(sdk,"libigl.zip"),"igl");
      Zip(output,Path.Combine(sdk,"cgal.zip"),"cgal");
      Zip(output,Path.Combine(sdk,"boost.zip"),"boost");
      var upstream=Path.Combine(root,"target/partuv-native/windows-source/extern");
      Folder(output,Path.Combine(upstream,"OpenABF/include/OpenABF"),"OpenABF/");
      Folder(output,Path.Combine(upstream,"easy_profiler/include/easy"),"easy/");
    }
    Console.WriteLine("Vendored header files: "+seen.Count);
  }
}
'@
$destination = Join-Path $RepositoryRoot 'native/uv/vendor/headers.zip'
[UvHeaderArchive]::Build($RepositoryRoot, $destination)
Get-FileHash -LiteralPath $destination -Algorithm SHA256
Get-Item -LiteralPath $destination | Select-Object Length

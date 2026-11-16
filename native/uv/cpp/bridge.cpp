// Native PartUV CPU + xatlas adapter. No Python ABI.
#include "pipeline.h"
#include <nlohmann/json.hpp>
#include <xatlas.h>
#include <Eigen/SVD>
#include <array>
#include <deque>
#include <numeric>
#include <memory>
#include <cstring>
using Json=nlohmann::json;
using UV=std::array<float,2>;
using Tri=std::array<uint32_t,3>;
using FaceUV=std::array<std::array<double,2>,3>;
using Progress=void(*)(int);
struct Chart { std::vector<int> ids; std::vector<UV> uv; std::vector<Tri> faces,valid; };
struct Packing { std::vector<FaceUV> uv; size_t charts; double utilization; double area=0.; };
struct Union {
 std::vector<int> p;
 Union(int n):p(n){std::iota(p.begin(),p.end(),0);}
 int root(int i){while(p[i]!=i){p[i]=p[p[i]];i=p[i];}return i;}
 void join(int a,int b){p[root(a)]=root(b);}
};
static Eigen::Vector2d point(const UV& p){return {p[0],p[1]};}
static double area(const Eigen::Vector2d&a,const Eigen::Vector2d&b,const Eigen::Vector2d&c){auto d=(b-a).eval();auto e=(c-a).eval();return std::abs(d.x()*e.y()-d.y()*e.x());}
static Packing pack(const std::vector<UV>& uv,const std::vector<Tri>& faces,int resolution,int padding,Progress progress){
 if(progress)progress(80);
 Union u((int)faces.size());std::map<std::pair<uint32_t,uint32_t>,int> edges;
 for(int i=0;i<(int)faces.size();++i)for(int c=0;c<3;++c){auto e=std::minmax(faces[i][c],faces[i][(c+1)%3]);auto k=std::make_pair(e.first,e.second);auto old=edges.find(k);if(old!=edges.end())u.join(i,old->second);else edges[k]=i;}
 std::vector<Chart> charts; std::map<int,size_t> group;
 for(int i=0;i<(int)faces.size();++i){int r=u.root(i);if(!group.count(r)){group[r]=charts.size();charts.emplace_back();}charts[group[r]].ids.push_back(i);}
 for(auto& ch:charts){
  std::set<uint32_t> unique;for(int i:ch.ids)unique.insert(faces[i].begin(),faces[i].end());std::map<uint32_t,uint32_t> local;
  for(uint32_t id:unique){local[id]=(uint32_t)ch.uv.size();ch.uv.push_back(uv[id]);}
  double largest=0.;std::vector<double> areas;
  for(int i:ch.ids){Tri f;for(int k=0;k<3;++k)f[k]=local[faces[i][k]];ch.faces.push_back(f);double a=area(point(ch.uv[f[0]]),point(ch.uv[f[1]]),point(ch.uv[f[2]]));areas.push_back(a);largest=std::max(largest,a);}
  for(size_t i=0;i<ch.faces.size();++i)if(areas[i]>std::max(largest*1e-10,1e-24))ch.valid.push_back(ch.faces[i]);
 }
 auto attempt=[&](bool rectangles,bool exhaustive){
  std::unique_ptr<xatlas::Atlas,void(*)(xatlas::Atlas*)> atlas(xatlas::Create(),xatlas::Destroy);
  std::vector<std::vector<UV>> sources;std::vector<std::vector<Tri>> triangles;
  for(auto& ch:charts){
   std::vector<UV> src=ch.uv;std::vector<Tri> fs=ch.valid;
   if(rectangles || fs.empty()){
    Eigen::Vector2f low=Eigen::Vector2f::Constant(INFINITY),high=Eigen::Vector2f::Constant(-INFINITY);
    for(auto& q:ch.uv){low=low.cwiseMin(Eigen::Vector2f(q[0],q[1]));high=high.cwiseMax(Eigen::Vector2f(q[0],q[1]));}
    float extent=std::max((high-low).maxCoeff(),1e-4f);high=high.cwiseMax((low.array()+extent*1e-4f).matrix());
    src={{low.x(),low.y()},{high.x(),low.y()},{high.x(),high.y()},{low.x(),high.y()}};fs={Tri{0,1,2},Tri{0,2,3}};
   }
   xatlas::UvMeshDecl decl;decl.vertexCount=(uint32_t)src.size();decl.vertexUvData=src.data();decl.vertexStride=sizeof(UV);decl.indexData=fs.data();decl.indexCount=(uint32_t)fs.size()*3;decl.indexFormat=xatlas::IndexFormat::UInt32;
   if(xatlas::AddUvMesh(atlas.get(),decl)!=xatlas::AddMeshError::Success)throw std::runtime_error("xatlas rejected UV mesh");
   sources.push_back(std::move(src));triangles.push_back(std::move(fs));
  }
  xatlas::PackOptions options;options.resolution=resolution;options.padding=padding;options.bruteForce=exhaustive;
  xatlas::Generate(atlas.get(),xatlas::ChartOptions{},options);
  if(atlas->atlasCount!=1 || atlas->meshCount!=charts.size())throw std::runtime_error("Packing requires exactly one atlas");
  double side=std::max(atlas->width,atlas->height);if(side<=0)throw std::runtime_error("Empty packed atlas");
  Packing out;out.uv.resize(faces.size());out.charts=charts.size();out.utilization=atlas->utilization[0]*atlas->width*atlas->height/(side*side);
  double sum_area=0.;
  for(size_t i=0;i<charts.size();++i){auto& ch=charts[i];auto& src=sources[i];auto& fs=triangles[i];auto& mesh=atlas->meshes[i];
   if(mesh.indexCount!=fs.size()*3)throw std::runtime_error("Packing changed face count");
   std::vector<std::array<Eigen::Vector2d,3>> after(fs.size());double maxarea=-1;size_t seed=0;
   for(size_t f=0;f<fs.size();++f){for(int c=0;c<3;++c){auto& v=mesh.vertexArray[mesh.indexArray[f*3+c]];if(v.xref!=fs[f][c])throw std::runtime_error("Packing changed face order");after[f][c]={v.uv[0]/side,v.uv[1]/side};}double a=area(point(src[fs[f][0]]),point(src[fs[f][1]]),point(src[fs[f][2]]));if(a>maxarea){maxarea=a;seed=f;}}
   auto a=point(src[fs[seed][0]]);auto b=after[seed][0];Eigen::Matrix2d A,B;
   A.col(0)=point(src[fs[seed][1]])-a;A.col(1)=point(src[fs[seed][2]])-a;
   B.col(0)=after[seed][1]-b;B.col(1)=after[seed][2]-b;
   if(std::abs(A.determinant())<1e-30)throw std::runtime_error("Degenerate chart anchor");
   Eigen::Matrix2d matrix=B*A.inverse();Eigen::Vector2d origin=a,destination=b;
   if(rectangles){Eigen::Vector2d low=Eigen::Vector2d::Constant(INFINITY),high=Eigen::Vector2d::Constant(-INFINITY);for(auto& p:src){low=low.cwiseMin(point(p));high=high.cwiseMax(point(p));}origin=(low+high)*.5;destination=matrix*(origin-a)+b;Eigen::JacobiSVD<Eigen::Matrix2d> svd(matrix,Eigen::ComputeFullU|Eigen::ComputeFullV);matrix=svd.matrixU()*svd.matrixV().transpose()*svd.singularValues().minCoeff();}
   Eigen::Matrix2d gram=matrix.transpose()*matrix;double scale=gram.trace()*.5;
   if(!matrix.allFinite() || scale<=0 || std::abs(gram(0,0)-scale)>scale*.006 || std::abs(gram(1,1)-scale)>scale*.006 || std::abs(gram(0,1))>scale*.003)throw std::runtime_error("Packing distorted chart");
   if(!rectangles)for(size_t f=0;f<fs.size();++f)if(area(point(src[fs[f][0]]),point(src[fs[f][1]]),point(src[fs[f][2]]))>std::max(maxarea*1e-6,1e-20))for(int c=0;c<3;++c)if((matrix*(point(src[fs[f][c]])-origin)+destination-after[f][c]).cwiseAbs().maxCoeff()>3e-5)throw std::runtime_error("Packing split continuous chart");
   for(size_t f=0;f<ch.faces.size();++f){std::array<Eigen::Vector2d,3> ps;for(int c=0;c<3;++c){ps[c]=matrix*(point(ch.uv[ch.faces[f][c]])-origin)+destination;if(!ps[c].allFinite() || ps[c].minCoeff()<-.00001 || ps[c].maxCoeff()>1.00001)throw std::runtime_error("Packed UV outside atlas");out.uv[ch.ids[f]][c]={ps[c].x(),ps[c].y()};}sum_area+=area(ps[0],ps[1],ps[2])*.5;}
  }
  if(sum_area>1.0001)throw std::runtime_error("Packed UV area exceeds atlas");
  out.area=sum_area;return out;
 };
 bool rectangles=false;Packing best;
 try{best=attempt(false,false);}catch(const std::exception&){rectangles=true;if(progress)progress(85);best=attempt(true,false);}
 // Search an additional placement for ordinary game assets. Always retain the
 // better valid atlas, so the extra search cannot degrade the default result.
 if(charts.size()<=256 && faces.size()<=100000){
  if(progress)progress(87);
  try{auto candidate=attempt(rectangles,true);if(candidate.area>best.area)best=std::move(candidate);}catch(const std::exception&){}
 }
 return best;
}
using Point=std::array<double,3>;
using TriangleKey=std::array<Point,3>;
static Json process(const Json& request,Progress progress){
 auto vertices=request.at("positions").get<std::vector<Point>>();auto faces=request.at("triangles").get<std::vector<Tri>>();
 std::vector<UV> uv;std::vector<Tri> uvfaces;
 if(request.contains("existing_uv")){
  auto existing=request.at("existing_uv").get<std::vector<FaceUV>>();if(existing.size()!=faces.size())throw std::runtime_error("Incomplete existing UV");
  using Key=std::tuple<uint32_t,float,float>;std::map<Key,uint32_t> unique;
  for(size_t i=0;i<faces.size();++i)for(int c=0;c<3;++c)unique[{faces[i][c],float(existing[i][c][0]),float(existing[i][c][1])}]=0;
  for(auto& kv:unique){kv.second=(uint32_t)uv.size();uv.push_back({std::get<1>(kv.first),std::get<2>(kv.first)});}
  for(size_t i=0;i<faces.size();++i){Tri f;for(int c=0;c<3;++c)f[c]=unique.at({faces[i][c],float(existing[i][c][0]),float(existing[i][c][1])});uvfaces.push_back(f);}
  for(auto& p:uv)if(!std::isfinite(p[0]) || !std::isfinite(p[1]))throw std::runtime_error("Non-finite source UV");
  auto packed=pack(uv,uvfaces,request.at("resolution"),request.at("padding"),progress);return Json{{"uv",packed.uv},{"charts",packed.charts},{"utilization",packed.utilization}};
 }
 Eigen::MatrixXd V(vertices.size(),3);Eigen::MatrixXi F(faces.size(),3);for(size_t i=0;i<vertices.size();++i)for(int c=0;c<3;++c)V(i,c)=vertices[i][c];for(size_t i=0;i<faces.size();++i)for(int c=0;c<3;++c)F(i,c)=faces[i][c];
 std::vector<NodeRecord> nodes;for(auto& v:request.at("tree"))nodes.push_back({v[0].get<int>(),v[1].get<int>(),v[2].get<int>()});
 std::vector<UVParts> parts;auto result=pipeline(V,F,nodes,"",request.at("threshold").get<double>(),false,parts).to_components();
 if(result.F.rows()!=(int)faces.size() || result.UV.rows()!=result.V.rows())throw std::runtime_error("PartUV changed mesh coverage");
 for(int i=0;i<result.UV.rows();++i)uv.push_back({float(result.UV(i,0)),float(result.UV(i,1))});
 for(int i=0;i<result.F.rows();++i)uvfaces.push_back({uint32_t(result.F(i,0)),uint32_t(result.F(i,1)),uint32_t(result.F(i,2))});
 for(auto& p:uv)if(!std::isfinite(p[0]) || !std::isfinite(p[1]))throw std::runtime_error("Non-finite PartUV result");
 auto packed=pack(uv,uvfaces,request.at("resolution"),request.at("padding"),progress);
 std::map<TriangleKey,std::deque<size_t>> original;
 for(size_t i=0;i<faces.size();++i){TriangleKey key={vertices[faces[i][0]],vertices[faces[i][1]],vertices[faces[i][2]]};std::sort(key.begin(),key.end());original[key].push_back(i);}
 std::vector<FaceUV> ordered(faces.size());
 for(size_t i=0;i<uvfaces.size();++i){TriangleKey points;for(int c=0;c<3;++c)for(int k=0;k<3;++k)points[c][k]=result.V(uvfaces[i][c],k);auto key=points;std::sort(key.begin(),key.end());auto found=original.find(key);if(found==original.end() || found->second.empty())throw std::runtime_error("PartUV changed geometry");auto fi=found->second.front();found->second.pop_front();for(int c=0;c<3;++c){int match=-1;for(int k=0;k<3;++k)if(points[k]==vertices[faces[fi][c]]){if(match>=0)throw std::runtime_error("Degenerate source triangle");match=k;}if(match<0)throw std::runtime_error("PartUV changed corner");ordered[fi][c]=packed.uv[i][match];}}
 for(auto& kv:original)if(!kv.second.empty())throw std::runtime_error("PartUV omitted triangles");
 return Json{{"uv",ordered},{"charts",packed.charts},{"utilization",packed.utilization}};
}
extern "C" {
struct NativeBuffer { char* data;size_t length;int error; };
NativeBuffer bevy_uv_process(const char* input,size_t length,Progress progress) noexcept {
 std::string data;int error=0;
 try {data=process(Json::parse(input,input+length),progress).dump();}catch(const std::exception& e){data=e.what();error=1;}catch(...){data="Unknown native UV failure";error=1;}
 char* buffer=static_cast<char*>(std::malloc(data.size()+1));if(!buffer)return {nullptr,0,1};std::memcpy(buffer,data.c_str(),data.size()+1);return {buffer,data.size(),error};
}
void bevy_uv_free(NativeBuffer b) noexcept {std::free(b.data);}
}

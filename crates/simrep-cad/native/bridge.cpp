#include <BRepTools.hxx>
#include <BRep_Builder.hxx>
#include <BRep_Tool.hxx>
#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepPrimAPI_MakeTorus.hxx>
#include <Bnd_Box.hxx>
#include <IGESCAFControl_Reader.hxx>
#include <IGESCAFControl_Writer.hxx>
#include <IGESControl_Controller.hxx>
#include <IGESData_IGESModel.hxx>
#include <IGESData_BasicEditor.hxx>
#include <Interface_Static.hxx>
#include <Poly_Triangulation.hxx>
#include <STEPCAFControl_Reader.hxx>
#include <STEPCAFControl_Writer.hxx>
#include <STEPControl_Controller.hxx>
#include <TColStd_SequenceOfAsciiString.hxx>
#include <TDataStd_Name.hxx>
#include <TDocStd_Document.hxx>
#include <TopExp_Explorer.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Shell.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>
#include <gp_Ax2.hxx>
#include <gp_Trsf.hxx>

#include <algorithm>
#include <cmath>
#include <cctype>
#include <cstdint>
#include <cstring>
#include <limits>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

namespace {
struct Body {
    TopoDS_Solid solid;
    TopoDS_Compound boundary;
    BRepClass3d_SolidClassifier classifier;
    BRepExtrema_DistShapeShape extrema;
    std::string name;
    double bounds[6];
    bool valid;

    Body(const TopoDS_Solid& s, std::string n) : solid(s), name(std::move(n)) {
        BRep_Builder builder;
        builder.MakeCompound(boundary);
        for (TopExp_Explorer face(solid, TopAbs_FACE); face.More(); face.Next())
            builder.Add(boundary, face.Current());
        valid = BRepCheck_Analyzer(solid).IsValid();
        if (valid) classifier.Load(solid);
        extrema.LoadS2(boundary); // Not the solid: interior-to-solid distance is zero.
        extrema.SetDeflection(1e-10);
        Bnd_Box box;
        BRepBndLib::Add(solid, box, false); // Geometric bounds, never triangulation bounds.
        box.Get(bounds[0], bounds[1], bounds[2], bounds[3], bounds[4], bounds[5]);
    }
};
struct Model { std::vector<std::unique_ptr<Body>> bodies; };
struct Triangles { std::vector<double> coordinates; };

void error(char* out, size_t size, const char* text) {
    if (size) {
        std::strncpy(out, text ? text : "OCCT operation failed", size - 1);
        out[size - 1] = 0;
    }
}

std::string name_of(const TDF_Label& label) {
    Handle(TDataStd_Name) attr;
    if (!label.FindAttribute(TDataStd_Name::GetID(), attr)) return "";
    const auto& text = attr->Get();
    std::string name(text.LengthOfCString() + 1, '\0');
    char* dest = name.data();
    name.resize(text.ToUTF8CString(dest));
    return name;
}

void add_leaf(Model& model, const Handle(XCAFDoc_ShapeTool)& tool,
              const TDF_Label& label, const TopLoc_Location& parent,
              const std::string& path, double scale) {
    TDF_Label definition = label;
    const bool reference = tool->GetReferredShape(label, definition);
    const auto location = parent * tool->GetLocation(label);
    std::string name = name_of(label);
    if (reference) {
        // XCAF default instance names are references such as "=>[0:1:1:2]".
        if (name.empty() || name.rfind("=>", 0) == 0) name = name_of(definition);
    }
    if (name.empty()) name = "part";
    const std::string full = path.empty() ? name : path + "/" + name;
    TDF_LabelSequence children;
    if (tool->GetComponents(definition, children)) {
        for (int i = 1; i <= children.Length(); ++i)
            add_leaf(model, tool, children.Value(i), location, full, scale);
        return;
    }
    auto shape = tool->GetShape(definition);
    // Definition location was already accumulated above.
    shape.Location(location);
    if (scale != 1.0) {
        gp_Trsf transform;
        transform.SetScale(gp_Pnt(0, 0, 0), scale);
        shape = BRepBuilderAPI_Transform(shape, transform, true).Shape();
    }
    size_t before = model.bodies.size();
    for (TopExp_Explorer solids(shape, TopAbs_SOLID); solids.More(); solids.Next()) {
        auto solid = TopoDS::Solid(solids.Current());
        std::string body_name = full;
        TDF_Label sub;
        if (tool->FindSubShape(definition, solids.Current(), sub)) {
            auto sub_name = name_of(sub);
            if (!sub_name.empty()) body_name += "/" + sub_name;
        }
        model.bodies.push_back(std::make_unique<Body>(solid, body_name));
    }
    if (model.bodies.size() == before) {
        // IGES commonly stores trimmed surfaces rather than solid entities.
        BRepBuilderAPI_Sewing sewing(1e-7);
        sewing.Add(shape);
        sewing.Perform();
        shape = sewing.SewedShape();
        if (!shape.IsNull() && shape.ShapeType() == TopAbs_FACE && sewing.NbFreeEdges() == 0) {
            BRep_Builder builder;
            TopoDS_Shell shell;
            builder.MakeShell(shell);
            builder.Add(shell, shape);
            shape = shell;
        }
        for (TopExp_Explorer shells(shape, TopAbs_SHELL); shells.More(); shells.Next()) {
            auto solid = BRepBuilderAPI_MakeSolid(TopoDS::Shell(shells.Current())).Solid();
            model.bodies.push_back(std::make_unique<Body>(solid, full));
        }
    }
    if (model.bodies.size() == before)
        throw std::runtime_error("CAD part has no solid or closed shell; export a closed B-rep");
    if (model.bodies.size() - before > 1)
        for (size_t i = before; i < model.bodies.size(); ++i)
            model.bodies[i]->name += "/body-" + std::to_string(i - before);
}

double unit_mm(std::string name) {
    std::transform(name.begin(), name.end(), name.begin(), ::tolower);
    if (name == "millimetre" || name == "millimeter" || name == "mm") return 1;
    if (name == "metre" || name == "meter" || name == "m") return 1000;
    if (name == "centimetre" || name == "centimeter" || name == "cm") return 10;
    if (name == "inch" || name == "in") return 25.4;
    if (name == "foot" || name == "ft") return 304.8;
    if (name == "micrometre" || name == "micron") return 0.001;
    throw std::runtime_error("Unit override requires one recognized source length unit");
}

// Original implementation of the complete-face measurement contract described in
// Iteration-CFD docs/16a-amagine-borrow.md, wall_min_tagged (Prosperity-3.0.0;
// its Amagine3D reference is Apache-2.0). No upstream code was copied or adapted.
// Provenance and upstream licence/NOTICE references: licenses/THIRD_PARTY.md.
TopoDS_Compound selected_faces(const Body& body, const size_t* ids, size_t count) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body.solid, TopAbs_FACE, faces);
    if (!count) throw std::runtime_error("CAD face selection is empty");
    TopoDS_Compound selected;
    BRep_Builder builder;
    builder.MakeCompound(selected);
    std::vector<bool> seen(faces.Extent(), false);
    for (size_t n = 0; n < count; ++n) {
        if (ids[n] >= static_cast<size_t>(faces.Extent()))
            throw std::runtime_error("CAD face index is out of range");
        if (seen[ids[n]]) throw std::runtime_error("duplicate CAD face index");
        seen[ids[n]] = true;
        builder.Add(selected, faces(ids[n] + 1));
    }
    TopTools_IndexedMapOfShape covered;
    TopExp::MapShapes(selected, TopAbs_FACE, covered);
    if (static_cast<size_t>(covered.Extent()) != count)
        throw std::runtime_error("CAD face coverage mismatch");
    return selected;
}

size_t support_face(const Body& body, const size_t* ids, size_t count,
                    const TopoDS_Shape& support) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body.solid, TopAbs_FACE, faces);
    for (size_t n = 0; n < count; ++n) {
        TopTools_IndexedMapOfShape members;
        TopExp::MapShapes(faces(ids[n] + 1), members);
        if (members.Contains(support)) return ids[n];
    }
    throw std::runtime_error("CAD closest-point support is outside selected faces");
}
} // namespace

extern "C" {
void* simrep_cad_read(const char* path, bool iges, double override_mm,
                      char* err, size_t size) {
    try {
        STEPControl_Controller::Init();
        Interface_Static::SetCVal("xstep.cascade.unit", "MM");
        Handle(TDocStd_Document) doc = new TDocStd_Document("BinXCAF");
        double scale = 1;
        if (iges) {
            IGESControl_Controller::Init();
            IGESCAFControl_Reader reader;
            reader.SetNameMode(true);
            if (reader.ReadFile(path) != IFSelect_RetDone)
                throw std::runtime_error("IGES read failed");
            if (override_mm > 0) {
                // GlobalSection::UnitValue includes CascadeUnit, despite old
                // headers describing it as meters. Use the explicit mm API.
                double source_mm = IGESData_BasicEditor::UnitFlagValue(
                    reader.IGESModel()->GlobalSection().UnitFlag());
                if (source_mm <= 0)
                    throw std::runtime_error("IGES unit override requires a recognized unit flag");
                scale = override_mm / source_mm;
            }
            if (!reader.Transfer(doc)) throw std::runtime_error("IGES transfer failed");
        } else {
            STEPCAFControl_Reader reader;
            reader.SetNameMode(true);
            reader.SetColorMode(false);
            reader.SetLayerMode(false);
            if (reader.ReadFile(path) != IFSelect_RetDone)
                throw std::runtime_error("STEP read failed");
            reader.ChangeReader().SetSystemLengthUnit(1.0);
            if (override_mm > 0) {
                TColStd_SequenceOfAsciiString lengths, angles, solids;
                reader.ChangeReader().FileUnits(lengths, angles, solids);
                if (lengths.Length() != 1)
                    throw std::runtime_error("Unit override is ambiguous for a mixed-unit STEP file");
                scale = override_mm / unit_mm(lengths.Value(1).ToCString());
            }
            if (!reader.Transfer(doc)) throw std::runtime_error("STEP transfer failed");
        }
        auto model = std::make_unique<Model>();
        auto tool = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
        TDF_LabelSequence roots;
        tool->GetFreeShapes(roots);
        for (int i = 1; i <= roots.Length(); ++i)
            add_leaf(*model, tool, roots.Value(i), TopLoc_Location(), "", scale);
        if (model->bodies.empty()) throw std::runtime_error("CAD file contains no solids");
        return model.release();
    } catch (const Standard_Failure& e) { error(err, size, e.GetMessageString()); }
      catch (const std::exception& e) { error(err, size, e.what()); }
    return nullptr;
}
void* simrep_cad_read_measurement(const char* path, bool iges, char* err, size_t size) {
    // OCCT's exchange diagnostics use cout independently of Rust's CLI output.
    // EXCHANGE serializes readers/writers while this scoped routing is active.
    struct DiagnosticStream {
        std::streambuf* previous = std::cout.rdbuf(std::cerr.rdbuf());
        ~DiagnosticStream() { std::cout.flush(); std::cout.rdbuf(previous); }
    } diagnostics;
    return simrep_cad_read(path, iges, 0.0, err, size);
}
void simrep_cad_free(void* model) { delete static_cast<Model*>(model); }
size_t simrep_cad_count(void* model) { return static_cast<Model*>(model)->bodies.size(); }
const char* simrep_cad_name(void* model, size_t i) {
    return static_cast<Model*>(model)->bodies.at(i)->name.c_str();
}
bool simrep_cad_valid(void* model, size_t i) {
    return static_cast<Model*>(model)->bodies.at(i)->valid;
}
uintptr_t simrep_cad_tshape(void* model, size_t i) {
    return reinterpret_cast<uintptr_t>(
        static_cast<Model*>(model)->bodies.at(i)->solid.TShape().get());
}
void simrep_cad_bounds(void* model, size_t i, double* bounds) {
    auto& body = *static_cast<Model*>(model)->bodies.at(i);
    std::copy(body.bounds, body.bounds + 6, bounds);
}
size_t simrep_cad_face_count(void* model, size_t i) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(static_cast<Model*>(model)->bodies.at(i)->solid, TopAbs_FACE, faces);
    return faces.Extent();
}
bool simrep_cad_measure(void* model, size_t a, const size_t* faces_a, size_t count_a,
                        size_t b, const size_t* faces_b, size_t count_b,
                        double* values, size_t* closest_faces, char* err, size_t size) {
    try {
        auto& left = *static_cast<Model*>(model)->bodies.at(a);
        auto& right = *static_cast<Model*>(model)->bodies.at(b);
        if (!left.valid || !right.valid)
            throw std::runtime_error("CAD measurement requires valid B-reps");
        auto selected_a = selected_faces(left, faces_a, count_a);
        auto selected_b = selected_faces(right, faces_b, count_b);
        BRepExtrema_DistShapeShape distance;
        distance.LoadS1(selected_a);
        distance.LoadS2(selected_b);
        distance.SetDeflection(1e-10);
        distance.Perform();
        if (!distance.IsDone() || distance.NbSolution() == 0 ||
            !std::isfinite(distance.Value()))
            throw std::runtime_error("CAD face-set minimum distance failed");
        const auto p = distance.PointOnShape1(1);
        const auto q = distance.PointOnShape2(1);
        const double result[] = {distance.Value(), p.X(), p.Y(), p.Z(), q.X(), q.Y(), q.Z()};
        std::copy(result, result + 7, values);
        closest_faces[0] = support_face(left, faces_a, count_a, distance.SupportOnShape1(1));
        closest_faces[1] = support_face(right, faces_b, count_b, distance.SupportOnShape2(1));
        return true;
    } catch (const Standard_Failure& e) { error(err, size, e.GetMessageString()); }
      catch (const std::exception& e) { error(err, size, e.what()); }
    return false;
}
void* simrep_cad_tessellate_faces(void* model, size_t i, const size_t* ids, size_t count,
                                 double tolerance, char* err, size_t size) {
    try {
        auto& body = *static_cast<Model*>(model)->bodies.at(i);
        auto selected = selected_faces(body, ids, count);
        // New measurement-only path: never reuse a coarser cached triangulation.
        BRepTools::Clean(selected);
        BRepMesh_IncrementalMesh mesher(selected, tolerance, false, 0.35, false);
        if (!mesher.IsDone()) throw std::runtime_error("CAD cross-check tessellation failed");
        auto result = std::make_unique<Triangles>();
        for (TopExp_Explorer it(selected, TopAbs_FACE); it.More(); it.Next()) {
            TopLoc_Location location;
            auto mesh = BRep_Tool::Triangulation(TopoDS::Face(it.Current()), location);
            if (mesh.IsNull() || mesh->NbTriangles() == 0)
                throw std::runtime_error("CAD cross-check does not cover every selected face");
            for (int n = 1; n <= mesh->NbTriangles(); ++n) {
                int a, b, c;
                mesh->Triangle(n).Get(a, b, c);
                for (int vertex : {a, b, c}) {
                    auto p = mesh->Node(vertex).Transformed(location.Transformation());
                    result->coordinates.insert(result->coordinates.end(), {p.X(), p.Y(), p.Z()});
                }
            }
        }
        return result.release();
    } catch (const Standard_Failure& e) { error(err, size, e.GetMessageString()); }
      catch (const std::exception& e) { error(err, size, e.what()); }
    return nullptr;
}
double simrep_cad_distance(void* model, size_t i, const double* p) {
    try {
        auto& body = *static_cast<Model*>(model)->bodies.at(i);
        const gp_Pnt point(p[0], p[1], p[2]);
        body.extrema.LoadS1(BRepBuilderAPI_MakeVertex(point).Vertex());
        if (!body.extrema.Perform() || !body.extrema.IsDone())
            return std::numeric_limits<double>::quiet_NaN();
        double d = body.extrema.Value();
        if (d <= 1e-12) return 0;
        body.classifier.Perform(point, 1e-12);
        auto state = body.classifier.State();
        if (state == TopAbs_IN) return -d;
        if (state == TopAbs_OUT || state == TopAbs_ON) return d;
    } catch (const Standard_Failure&) {
        return std::numeric_limits<double>::quiet_NaN();
    } catch (const std::exception&) {
        return std::numeric_limits<double>::quiet_NaN();
    }
    return std::numeric_limits<double>::quiet_NaN();
}
void* simrep_cad_tessellate(void* model, size_t i, double tolerance,
                           bool parallel, char* err, size_t size) {
    try {
        auto& body = *static_cast<Model*>(model)->bodies.at(i);
        // Parallel face meshing. Output is compared against the serial mesher
        // before it is trusted; the deflection arguments are unchanged.
        BRepMesh_IncrementalMesh mesher(body.solid, tolerance, false, 0.35, parallel);
        if (!mesher.IsDone()) throw std::runtime_error("OCCT tessellation failed");
        auto result = std::make_unique<Triangles>();
        for (TopExp_Explorer it(body.solid, TopAbs_FACE); it.More(); it.Next()) {
            const auto face = TopoDS::Face(it.Current());
            TopLoc_Location location;
            auto mesh = BRep_Tool::Triangulation(face, location);
            if (mesh.IsNull()) throw std::runtime_error("OCCT face has no triangulation");
            for (int n = 1; n <= mesh->NbTriangles(); ++n) {
                int a, b, c;
                mesh->Triangle(n).Get(a, b, c);
                if (face.Orientation() == TopAbs_REVERSED) std::swap(b, c);
                for (int vertex : {a, b, c}) {
                    auto p = mesh->Node(vertex).Transformed(location.Transformation());
                    result->coordinates.insert(result->coordinates.end(), {p.X(), p.Y(), p.Z()});
                }
            }
        }
        return result.release();
    } catch (const Standard_Failure& e) { error(err, size, e.GetMessageString()); }
      catch (const std::exception& e) { error(err, size, e.what()); }
    return nullptr;
}
size_t simrep_cad_triangle_values(void* p) {
    return static_cast<Triangles*>(p)->coordinates.size();
}
const double* simrep_cad_triangle_data(void* p) {
    return static_cast<Triangles*>(p)->coordinates.data();
}
void simrep_cad_triangles_free(void* p) { delete static_cast<Triangles*>(p); }

static std::vector<double> collect_triangles(const TopoDS_Shape& solid) {
    std::vector<double> coordinates;
    for (TopExp_Explorer it(solid, TopAbs_FACE); it.More(); it.Next()) {
        const auto face = TopoDS::Face(it.Current());
        TopLoc_Location location;
        auto mesh = BRep_Tool::Triangulation(face, location);
        if (mesh.IsNull()) continue;
        for (int n = 1; n <= mesh->NbTriangles(); ++n) {
            int a, b, c;
            mesh->Triangle(n).Get(a, b, c);
            if (face.Orientation() == TopAbs_REVERSED) std::swap(b, c);
            for (int vertex : {a, b, c}) {
                auto p = mesh->Node(vertex).Transformed(location.Transformation());
                coordinates.insert(coordinates.end(), {p.X(), p.Y(), p.Z()});
            }
        }
    }
    return coordinates;
}

// Returns true when parallel face meshing reproduces the serial triangulation
// exactly, including order. Used to decide whether the parallel mesher is safe.
bool simrep_cad_parallel_tessellation_matches(void* model, size_t i, double tolerance,
                                              char* err, size_t size) {
    try {
        auto& body = *static_cast<Model*>(model)->bodies.at(i);
        BRepTools::Clean(body.solid);
        BRepMesh_IncrementalMesh serial(body.solid, tolerance, false, 0.35, false);
        if (!serial.IsDone()) throw std::runtime_error("serial tessellation failed");
        auto serial_coords = collect_triangles(body.solid);
        BRepTools::Clean(body.solid);
        BRepMesh_IncrementalMesh parallel(body.solid, tolerance, false, 0.35, true);
        if (!parallel.IsDone()) throw std::runtime_error("parallel tessellation failed");
        auto parallel_coords = collect_triangles(body.solid);
        return serial_coords == parallel_coords;
    } catch (const Standard_Failure& e) { error(err, size, e.GetMessageString()); }
      catch (const std::exception& e) { error(err, size, e.what()); }
    return false;
}

// Owned fixtures, generated by the same exchange writers as production CAD tools.
bool simrep_cad_fixture(const char* path, int kind, const char* schema,
                        const char* unit, char* err, size_t size) {
    try {
        STEPControl_Controller::Init();
        Interface_Static::SetCVal("xstep.cascade.unit", "MM");
        Interface_Static::SetCVal("write.step.schema", schema);
        Interface_Static::SetCVal("write.step.unit", unit);
        Handle(TDocStd_Document) doc = new TDocStd_Document("BinXCAF");
        auto tool = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
        TopoDS_Shape shape;
        std::string name;
        if (kind == 0) {
            shape = BRepPrimAPI_MakeSphere(0.31).Shape();
            name = "sphere";
        } else if (kind == 1) {
            shape = BRepPrimAPI_MakeCylinder(
                gp_Ax2(gp_Pnt(0, 0, -2), gp_Dir(0, 0, 1)), 0.31, 4).Shape();
            name = "cylinder";
        } else if (kind == 2) {
            shape = BRepPrimAPI_MakeTorus(0.55, 0.19).Shape();
            name = "ring";
        } else {
            BRep_Builder builder;
            TopoDS_Compound pair;
            builder.MakeCompound(pair);
            auto sphere = BRepPrimAPI_MakeSphere(0.31).Shape();
            const double offset = kind == 4 ? 0.31 : 0.6;
            for (double x : {-offset, offset}) {
                gp_Trsf transform;
                transform.SetTranslation(gp_Vec(x, 0, 0));
                builder.Add(pair, sphere.Moved(TopLoc_Location(transform)));
            }
            shape = pair;
            name = kind == 4 ? "touching" : "pair";
        }
        auto label = tool->AddShape(shape, kind == 3 || kind == 4);
        TDataStd_Name::Set(label, TCollection_ExtendedString(name.c_str()));
        if (kind == 3 || kind == 4) {
            TDF_LabelSequence children;
            tool->GetComponents(label, children);
            TDataStd_Name::Set(children.Value(1), TCollection_ExtendedString("left"));
            TDataStd_Name::Set(children.Value(2), TCollection_ExtendedString("right"));
        }
        std::string filename(path);
        bool iges = filename.size() >= 4 &&
            (filename.substr(filename.size()-4) == ".igs" ||
             filename.substr(filename.size()-5) == ".iges");
        if (iges) {
            IGESControl_Controller::Init();
            Interface_Static::SetIVal("write.iges.brep.mode", 1);
            Interface_Static::SetCVal("write.iges.unit", unit);
            IGESCAFControl_Writer writer;
            if (!writer.Perform(doc, path)) throw std::runtime_error("IGES fixture write failed");
        } else {
            STEPCAFControl_Writer writer;
            if (!writer.Transfer(doc, STEPControl_AsIs) ||
                writer.Write(path) != IFSelect_RetDone)
                throw std::runtime_error("STEP fixture write failed");
        }
        return true;
    } catch (const Standard_Failure& e) { error(err, size, e.GetMessageString()); }
      catch (const std::exception& e) { error(err, size, e.what()); }
    return false;
}
} // extern C

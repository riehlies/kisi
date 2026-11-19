"""The 3D picture of fdtd-field-view.md: a VTK file of validation/fdtd/volume_fields.py (vtk subcommand) rendered
off-screen with VTK (pip: vtk) - metal mask contour, one |E| isosurface, streamlines of S from port 1's trace.
python3 vtk3d.py field.vtk out.png"""
import sys, vtk
r = vtk.vtkRectilinearGridReader(); r.SetFileName(sys.argv[1]); r.ReadAllScalarsOn(); r.ReadAllVectorsOn(); r.Update()
g = r.GetOutput()
ren = vtk.vtkRenderer(); ren.SetBackground(1, 1, 1)
# metal: isosurface of the mask
g.GetPointData().SetActiveScalars("metal")
c = vtk.vtkContourFilter(); c.SetInputData(g); c.SetValue(0, 0.5)
m = vtk.vtkPolyDataMapper(); m.SetInputConnection(c.GetOutputPort()); m.ScalarVisibilityOff()
a = vtk.vtkActor(); a.SetMapper(m); a.GetProperty().SetColor(0.80, 0.50, 0.20); a.GetProperty().SetOpacity(0.35); ren.AddActor(a)
# |E| isosurface
g.GetPointData().SetActiveScalars("E_abs")
c2 = vtk.vtkContourFilter(); c2.SetInputData(g); c2.SetValue(0, 2.0e4)
m2 = vtk.vtkPolyDataMapper(); m2.SetInputConnection(c2.GetOutputPort()); m2.ScalarVisibilityOff()
a2 = vtk.vtkActor(); a2.SetMapper(m2); a2.GetProperty().SetColor(0.35, 0.2, 0.6); a2.GetProperty().SetOpacity(0.45); ren.AddActor(a2)
# streamlines of S from seeds across the input microstrip
g.GetPointData().SetActiveVectors("S")
seeds = vtk.vtkLineSource(); seeds.SetPoint1(-3.5, -0.25, 1.62); seeds.SetPoint2(-3.5, 0.25, 1.75); seeds.SetResolution(12)
st = vtk.vtkStreamTracer(); st.SetInputData(g); st.SetSourceConnection(seeds.GetOutputPort()); st.SetMaximumPropagation(30); st.SetIntegrationDirectionToForward(); st.SetIntegratorTypeToRungeKutta45()
tb = vtk.vtkTubeFilter(); tb.SetInputConnection(st.GetOutputPort()); tb.SetRadius(0.012)
m3 = vtk.vtkPolyDataMapper(); m3.SetInputConnection(tb.GetOutputPort()); m3.ScalarVisibilityOff()
a3 = vtk.vtkActor(); a3.SetMapper(m3); a3.GetProperty().SetColor(0.1, 0.55, 0.5); ren.AddActor(a3)
cam = ren.GetActiveCamera(); cam.SetFocalPoint(0, 0, 0.8); cam.SetPosition(-7, -9, 7); cam.SetViewUp(0, 0, 1); ren.ResetCameraClippingRange(); cam.Zoom(1.6)
rw = vtk.vtkRenderWindow(); rw.SetOffScreenRendering(1); rw.AddRenderer(ren); rw.SetSize(1200, 800); rw.Render()
w2i = vtk.vtkWindowToImageFilter(); w2i.SetInput(rw); w2i.Update()
pw = vtk.vtkPNGWriter(); pw.SetFileName(sys.argv[2]); pw.SetInputConnection(w2i.GetOutputPort()); pw.Write()
print("ok", st.GetOutput().GetNumberOfPoints())

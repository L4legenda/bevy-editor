extends SceneTree
## Runs only inside an isolated generated project. Never opens the source game.

const Probe = preload("res://probe_node.gd")
var results: Dictionary = {"engine": Engine.get_version_info(), "checks": [], "benchmarks": []}
var failures: int = 0

func _initialize() -> void:
	call_deferred("run")

func check(label: String, passed: bool, detail: Variant = null) -> void:
	results.checks.append({"name": label, "passed": passed, "detail": detail})
	if not passed:
		failures += 1
		printerr("FAILED: ", label, " ", detail)

func text(path: String, value: String) -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	file.store_string(value)
	file.close()

func save_node(node: Node, path: String, flags: int = 0) -> PackedScene:
	var packed := PackedScene.new()
	check("pack " + path, packed.pack(node) == OK)
	check("save " + path, ResourceSaver.save(packed, path, flags) == OK)
	return packed

func load_fresh(path: String) -> PackedScene:
	return ResourceLoader.load(path, "PackedScene", ResourceLoader.CACHE_MODE_IGNORE_DEEP) as PackedScene

func topology_and_ownership() -> void:
	var node := Node3D.new()
	node.name = "Root"
	var owned := Node3D.new()
	owned.name = "Owned"
	node.add_child(owned)
	owned.owner = node
	var runtime_child := Node3D.new()
	runtime_child.name = "RuntimeOnly"
	node.add_child(runtime_child)
	var scene := save_node(node, "res://ownership.tscn")
	var restored := scene.instantiate()
	check("ownership excludes unowned runtime nodes", restored.get_child_count() == 1)
	node.free()
	restored.free()

func references_and_roundtrip() -> void:
	var node := Probe.new()
	node.name = "Персонаж_猫"
	node.position = Vector3(1.125, -2.5, 3.75)
	node.power = 9.125
	var hand := Node3D.new()
	hand.name = "Hand"
	node.add_child(hand)
	hand.owner = node
	node.target = NodePath("Hand")
	save_node(node, "res://roundtrip.tscn")
	var restored := load_fresh("res://roundtrip.tscn").instantiate()
	check("unicode and float roundtrip", restored.name == node.name and restored.position == node.position and restored.power == node.power)
	check("node reference resolves before rename", restored.get_node_or_null(restored.target) != null)
	restored.get_node("Hand").name = "Palm"
	check("runtime rename leaves a stored NodePath stale", restored.get_node_or_null(restored.target) == null)
	var original := FileAccess.get_file_as_bytes("res://roundtrip.tscn")
	save_node(node, "res://roundtrip.tscn")
	check("unchanged instance saves deterministically", original == FileAccess.get_file_as_bytes("res://roundtrip.tscn"))
	var source := FileAccess.get_file_as_string("res://roundtrip.tscn")
	check("default values omitted from source", not source.contains("scale ="))
	text("res://unknown.tscn", source + "\nunknown_future_property = 123\n")
	var future := load_fresh("res://unknown.tscn")
	results["unknown_property_accepted"] = future != null
	if future != null:
		var future_node := future.instantiate()
		save_node(future_node, "res://unknown_saved.tscn")
		results["unknown_property_preserved"] = FileAccess.get_file_as_string("res://unknown_saved.tscn").contains("unknown_future_property")
		future_node.free()
	node.free()
	restored.free()

func resource_sharing() -> void:
	for local: bool in [false, true]:
		var node := Node3D.new()
		node.name = "Root"
		var mesh_node := MeshInstance3D.new()
		mesh_node.name = "Mesh"
		mesh_node.mesh = BoxMesh.new()
		var material := StandardMaterial3D.new()
		material.resource_local_to_scene = local
		mesh_node.material_override = material
		node.add_child(mesh_node)
		mesh_node.owner = node
		var path := "res://sharing_%s.tscn" % local
		var scene := save_node(node, path)
		var first := scene.instantiate()
		var second := scene.instantiate()
		var a: Material = first.get_node("Mesh").material_override
		var b: Material = second.get_node("Mesh").material_override
		check("resource identity local=%s" % local, (a == b) == not local)
		a.albedo_color = Color.RED
		check("material mutation isolation local=%s" % local, (b.albedo_color == Color.RED) == not local)
		node.free()
		first.free()
		second.free()

func composition_and_overrides() -> void:
	var base := Probe.new()
	base.name = "Base"
	var transient := save_node(base, "res://base.tscn")
	results["saved_in_memory_resource_path"] = transient.resource_path
	var base_scene := load_fresh("res://base.tscn")
	var root := Node3D.new()
	root.name = "Level"
	var inherited := base_scene.instantiate()
	inherited.name = "Inherited"
	root.add_child(inherited)
	inherited.owner = root
	var customized := base_scene.instantiate()
	customized.name = "Customized"
	customized.power = 3.0
	root.add_child(customized)
	customized.owner = root
	save_node(root, "res://composed.tscn")
	check("composition stores an instance reference", FileAccess.get_file_as_string("res://composed.tscn").contains("instance=ExtResource"))
	var cached_a := ResourceLoader.load("res://base.tscn")
	var cached_b := ResourceLoader.load("res://base.tscn")
	check("resource loader reuses cached identity", cached_a == cached_b)
	base.power = 7.0
	save_node(base, "res://base.tscn")
	var fresh := load_fresh("res://composed.tscn").instantiate()
	check("unmodified instance inherits base updates", fresh.get_node("Inherited").power == 7.0, fresh.get_node("Inherited").power)
	check("explicit override survives base updates", fresh.get_node("Customized").power == 3.0, fresh.get_node("Customized").power)
	base.free()
	root.free()
	fresh.free()

func median(values: Array) -> float:
	values.sort()
	return float(values[values.size() / 2])

func benchmark(count: int) -> void:
	var root := Node3D.new()
	root.name = "Scene"
	for i in count:
		var child := Node3D.new()
		child.name = "Entity_%d" % i
		child.position = Vector3(i % 100, i / 100, -i % 7)
		root.add_child(child)
		child.owner = root
	var packed := PackedScene.new()
	var started := Time.get_ticks_usec()
	packed.pack(root)
	var packing_us := Time.get_ticks_usec() - started
	for extension: String in ["tscn", "scn", "compressed.scn"]:
		var path := "res://bench_%d.%s" % [count, extension]
		var flags := ResourceSaver.FLAG_COMPRESS if extension == "compressed.scn" else 0
		started = Time.get_ticks_usec()
		var error := ResourceSaver.save(packed, path, flags)
		check("benchmark save %s" % path, error == OK)
		var save_us := Time.get_ticks_usec() - started
		var load_times: Array = []
		var instance_times: Array = []
		# Warm filesystem pages; ignore the engine resource cache on every load.
		for sample in 8:
			started = Time.get_ticks_usec()
			var loaded := load_fresh(path)
			var load_us := Time.get_ticks_usec() - started
			started = Time.get_ticks_usec()
			var instance := loaded.instantiate()
			var instance_us := Time.get_ticks_usec() - started
			check("benchmark instance count %s sample %d" % [path, sample], instance.get_child_count() == count)
			instance.free()
			if sample > 0:
				load_times.append(load_us)
				instance_times.append(instance_us)
		results.benchmarks.append({"entities": count, "format": extension, "bytes": FileAccess.get_file_as_bytes(path).size(), "pack_us": packing_us, "save_us": save_us, "load_median_us": median(load_times), "instantiate_median_us": median(instance_times), "samples": 7, "load_samples_us": load_times, "instantiate_samples_us": instance_times})
	root.free()

func run() -> void:
	topology_and_ownership()
	references_and_roundtrip()
	resource_sharing()
	composition_and_overrides()
	for count: int in [100, 1000, 10000]:
		benchmark(count)
	results["failures"] = failures
	text("res://results.json", JSON.stringify(results, "  "))
	print("SCENE_LAB_RESULT ", JSON.stringify({"failures": failures, "checks": results.checks.size(), "benchmarks": results.benchmarks.size()}))
	quit(0 if failures == 0 else 1)

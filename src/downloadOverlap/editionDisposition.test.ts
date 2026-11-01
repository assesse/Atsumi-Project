import {describe,it,expect} from 'vitest';
import {planEditionDisposition,keepEditions,type EditionDisposition} from './editionDisposition';
describe('edition disposition drafts',()=>{
 it('keeps or excludes the explicitly chosen edition without reading retained draft selections',()=>{
  const pages={existing:[3],incoming:[4]};
  expect(planEditionDisposition(keepEditions(),pages,'incoming')).toEqual({kind:'decision',action:'keep_both_continue'});
  expect(planEditionDisposition({existing:'remove',incoming:'keep'},pages,'incoming')).toEqual({kind:'decision',action:'remove_existing_continue'});
  expect(planEditionDisposition({existing:'keep',incoming:'remove'},pages,'incoming')).toEqual({kind:'decision',action:'remove_incoming'});
 });
 it('merges only the partial donor into an intact retained edition in either direction',()=>{
  for(const source of ['existing','incoming'] as const){const target=source==='incoming'?'existing':'incoming';
   const modes={existing:'keep',incoming:'keep'} as Record<typeof source,EditionDisposition>;modes[source]='partial';
   expect(planEditionDisposition(modes,{existing:[1],incoming:[2]},'incoming')).toEqual({kind:'merge',sourceSide:source,sourcePages:source==='incoming'?[2]:[1],selectedPages:{[source]:source==='incoming'?[2]:[1],[target]:[]},targetSelectionOnly:false});
  }
 });
 it('preserves only explicit pages when both editions are partial or the other is removed',()=>{
  expect(planEditionDisposition({existing:'partial',incoming:'partial'},{existing:[1],incoming:[2]},'incoming')).toMatchObject({sourceSide:'existing',targetSelectionOnly:true,selectedPages:{existing:[1],incoming:[2]}});
  expect(planEditionDisposition({existing:'partial',incoming:'remove'},{existing:[1],incoming:[2]},'incoming')).toMatchObject({sourceSide:'incoming',sourcePages:[],targetSelectionOnly:true,selectedPages:{existing:[1],incoming:[]}});
 });
 it('rejects empty partial selection and removing every edition without mutating saved choices',()=>{
  const pages={existing:[1],incoming:[]};const before=structuredClone(pages);
  expect(()=>planEditionDisposition({existing:'keep',incoming:'partial'},pages,'incoming')).toThrow('선택');
  expect(()=>planEditionDisposition({existing:'remove',incoming:'remove'},pages,'incoming')).toThrow('하나');
  expect(pages).toEqual(before);
 });
});
